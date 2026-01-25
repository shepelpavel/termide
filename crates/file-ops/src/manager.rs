//! Central operation manager for coordinating file operations.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use termide_vfs::VfsManager;

use crate::queue::{OperationQueue, QueuedOperation};
use crate::retry::RetryPolicy;
use crate::types::{
    OperationControl, OperationError, OperationEvent, OperationId, OperationInfo, OperationPath,
    OperationPriority, OperationProgress, OperationRequest, OperationResult, OperationType,
};
use crate::worker::{
    DownloadWorker, LocalCopyWorker, LocalDeleteWorker, OperationWorker, UploadWorker,
};

/// Configuration for the operation manager.
#[derive(Debug, Clone)]
pub struct OperationManagerConfig {
    /// Maximum concurrent operations.
    pub max_concurrent: usize,
    /// Maximum queue size (0 = unlimited).
    pub max_queue_size: usize,
    /// Default retry policy for network operations.
    pub retry_policy: RetryPolicy,
}

impl Default for OperationManagerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 2,
            max_queue_size: 100,
            retry_policy: RetryPolicy::network(),
        }
    }
}

/// Active operation state.
struct ActiveOperation {
    /// Operation information.
    info: OperationInfo,
    /// Worker thread handle.
    thread_handle: Option<JoinHandle<OperationResult>>,
    /// Progress receiver.
    progress_rx: mpsc::Receiver<OperationProgress>,
}

/// Central manager for all file operations.
pub struct OperationManager {
    /// Configuration.
    config: OperationManagerConfig,
    /// VFS manager for remote operations.
    vfs_manager: Arc<VfsManager>,
    /// Operation queue.
    queue: OperationQueue,
    /// Active operations.
    active: HashMap<OperationId, ActiveOperation>,
    /// Event channel sender.
    event_tx: mpsc::Sender<OperationEvent>,
    /// Event channel receiver.
    event_rx: mpsc::Receiver<OperationEvent>,
    /// Next operation ID.
    next_id: AtomicU64,
}

impl OperationManager {
    /// Create a new operation manager.
    pub fn new(vfs_manager: Arc<VfsManager>) -> Self {
        Self::with_config(vfs_manager, OperationManagerConfig::default())
    }

    /// Create with custom configuration.
    pub fn with_config(vfs_manager: Arc<VfsManager>, config: OperationManagerConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            queue: OperationQueue::with_max_size(config.max_queue_size),
            config,
            vfs_manager,
            active: HashMap::new(),
            event_tx,
            event_rx,
            next_id: AtomicU64::new(1),
        }
    }

    /// Generate a new unique operation ID.
    fn generate_id(&self) -> OperationId {
        OperationId::new(self.next_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Queue a new operation.
    ///
    /// Returns the operation ID if queued successfully.
    pub fn queue_operation(
        &mut self,
        request: OperationRequest,
    ) -> Result<OperationId, OperationError> {
        let id = self.generate_id();
        let queued = QueuedOperation::new(id, request);

        if !self.queue.enqueue(queued) {
            return Err(OperationError::QueueFull);
        }

        Ok(id)
    }

    /// Start a queued operation immediately (bypassing the queue).
    pub fn start_now(&mut self, request: OperationRequest) -> Result<OperationId, OperationError> {
        let id = self.generate_id();
        self.start_operation(id, request)?;
        Ok(id)
    }

    /// Start a specific queued operation.
    pub fn start(&mut self, id: OperationId) -> Result<(), OperationError> {
        let queued = self.queue.remove(id).ok_or(OperationError::NotFound(id))?;

        self.start_operation(id, queued.request)
    }

    /// Internal: start an operation.
    fn start_operation(
        &mut self,
        id: OperationId,
        request: OperationRequest,
    ) -> Result<(), OperationError> {
        // Check concurrent limit
        if self.active.len() >= self.config.max_concurrent {
            // Re-queue with high priority
            let mut queued = QueuedOperation::new(id, request);
            queued.priority = OperationPriority::High;
            self.queue.enqueue(queued);
            return Ok(());
        }

        let control = OperationControl::new();
        let (progress_tx, progress_rx) = mpsc::channel();

        // Create worker based on operation type
        let worker: Box<dyn OperationWorker> = self.create_worker(&request)?;

        // Clone what we need for the thread
        let control_clone = control.clone();
        let event_tx = self.event_tx.clone();

        // Start worker thread
        let thread_handle = thread::spawn(move || {
            let mut worker = worker;
            let result = worker.execute(&control_clone, &progress_tx);

            // Send completion event
            let _ = event_tx.send(OperationEvent::Completed(id, result.clone()));

            result
        });

        // Send started event
        let _ = self.event_tx.send(OperationEvent::Started(id));

        // Store active operation
        let info = OperationInfo {
            id,
            op_type: request.op_type,
            sources: request.sources,
            destination: request.destination,
            progress: OperationProgress::default(),
            control,
            is_active: true,
        };

        self.active.insert(
            id,
            ActiveOperation {
                info,
                thread_handle: Some(thread_handle),
                progress_rx,
            },
        );

        Ok(())
    }

    /// Create a worker for the given request.
    fn create_worker(
        &self,
        request: &OperationRequest,
    ) -> Result<Box<dyn OperationWorker>, OperationError> {
        match request.op_type {
            OperationType::Copy | OperationType::Move => {
                // Determine if local or remote
                let all_local = request.sources.iter().all(|p| p.is_local())
                    && request
                        .destination
                        .as_ref()
                        .map(|d| d.is_local())
                        .unwrap_or(true);

                if all_local {
                    let sources: Vec<PathBuf> = request
                        .sources
                        .iter()
                        .filter_map(|p| match p {
                            OperationPath::Local(path) => Some(path.clone()),
                            _ => None,
                        })
                        .collect();

                    let destination = match &request.destination {
                        Some(OperationPath::Local(path)) => path.clone(),
                        _ => {
                            return Err(OperationError::Invalid(
                                "Copy/Move requires destination".to_string(),
                            ))
                        }
                    };

                    Ok(Box::new(LocalCopyWorker::new(
                        sources,
                        destination,
                        request.is_move,
                    )))
                } else {
                    Err(OperationError::Invalid(
                        "Cross-protocol copy not yet supported".to_string(),
                    ))
                }
            }

            OperationType::Delete => {
                let all_local = request.sources.iter().all(|p| p.is_local());

                if all_local {
                    let paths: Vec<PathBuf> = request
                        .sources
                        .iter()
                        .filter_map(|p| match p {
                            OperationPath::Local(path) => Some(path.clone()),
                            _ => None,
                        })
                        .collect();

                    Ok(Box::new(LocalDeleteWorker::new(paths)))
                } else {
                    Err(OperationError::Invalid(
                        "Remote delete not yet supported via manager".to_string(),
                    ))
                }
            }

            OperationType::Download => {
                let remote = match request.sources.first() {
                    Some(OperationPath::Remote(path)) => path.clone(),
                    _ => {
                        return Err(OperationError::Invalid(
                            "Download requires remote source".to_string(),
                        ))
                    }
                };

                let local = match &request.destination {
                    Some(OperationPath::Local(path)) => path.clone(),
                    _ => {
                        return Err(OperationError::Invalid(
                            "Download requires local destination".to_string(),
                        ))
                    }
                };

                Ok(Box::new(DownloadWorker::new(
                    Arc::clone(&self.vfs_manager),
                    remote,
                    local,
                )))
            }

            OperationType::Upload => {
                let local = match request.sources.first() {
                    Some(OperationPath::Local(path)) => path.clone(),
                    _ => {
                        return Err(OperationError::Invalid(
                            "Upload requires local source".to_string(),
                        ))
                    }
                };

                let remote = match &request.destination {
                    Some(OperationPath::Remote(path)) => path.clone(),
                    _ => {
                        return Err(OperationError::Invalid(
                            "Upload requires remote destination".to_string(),
                        ))
                    }
                };

                Ok(Box::new(UploadWorker::new(
                    Arc::clone(&self.vfs_manager),
                    local,
                    remote,
                )))
            }
        }
    }

    /// Pause an active operation.
    pub fn pause(&self, id: OperationId) {
        if let Some(op) = self.active.get(&id) {
            op.info.control.set_paused(true);
            let _ = self.event_tx.send(OperationEvent::Paused(id));
        }
    }

    /// Resume a paused operation.
    pub fn resume(&self, id: OperationId) {
        if let Some(op) = self.active.get(&id) {
            op.info.control.set_paused(false);
            let _ = self.event_tx.send(OperationEvent::Resumed(id));
        }
    }

    /// Cancel an operation (active or queued).
    pub fn cancel(&mut self, id: OperationId) {
        // Try to cancel active operation
        if let Some(op) = self.active.get(&id) {
            op.info.control.cancel();
            // The thread will send the completion event
        }

        // Try to remove from queue
        self.queue.remove(id);
    }

    /// Poll for events and update state.
    ///
    /// Returns all pending events.
    pub fn poll(&mut self) -> Vec<OperationEvent> {
        let mut events = Vec::new();

        // Collect events from the channel
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }

        // Update progress for active operations
        let mut completed = Vec::new();
        for (id, op) in &mut self.active {
            // Drain progress updates
            while let Ok(progress) = op.progress_rx.try_recv() {
                events.push(OperationEvent::Progress(*id, progress.clone()));
                op.info.progress = progress;
            }

            // Check if thread completed
            if let Some(handle) = op.thread_handle.take() {
                if handle.is_finished() {
                    // Thread finished, get result
                    match handle.join() {
                        Ok(result) => {
                            events.push(OperationEvent::Completed(*id, result));
                            completed.push(*id);
                        }
                        Err(_) => {
                            events.push(OperationEvent::Completed(
                                *id,
                                OperationResult::Failed("Thread panicked".to_string()),
                            ));
                            completed.push(*id);
                        }
                    }
                } else {
                    // Put handle back
                    op.thread_handle = Some(handle);
                }
            }
        }

        // Remove completed operations
        for id in completed {
            self.active.remove(&id);
        }

        // Start queued operations if we have capacity
        while self.active.len() < self.config.max_concurrent {
            if let Some(queued) = self.queue.dequeue() {
                let _ = self.start_operation(queued.id, queued.request);
            } else {
                break;
            }
        }

        events
    }

    /// Get information about an operation.
    pub fn get_info(&self, id: OperationId) -> Option<&OperationInfo> {
        self.active.get(&id).map(|op| &op.info)
    }

    /// Get all active operation IDs.
    pub fn active_ids(&self) -> Vec<OperationId> {
        self.active.keys().copied().collect()
    }

    /// Get all queued operation IDs.
    pub fn queued_ids(&self) -> Vec<OperationId> {
        self.queue.ids()
    }

    /// Get the number of active operations.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Get the number of queued operations.
    pub fn queued_count(&self) -> usize {
        self.queue.len()
    }

    /// Check if any operations are running or queued.
    pub fn has_operations(&self) -> bool {
        !self.active.is_empty() || !self.queue.is_empty()
    }

    /// Cancel all operations and clear the queue.
    pub fn cancel_all(&mut self) {
        // Cancel active operations
        for op in self.active.values() {
            op.info.control.cancel();
        }

        // Clear queue
        self.queue.clear();
    }

    /// Change priority of a queued operation.
    pub fn set_priority(&mut self, id: OperationId, priority: OperationPriority) -> bool {
        self.queue.set_priority(id, priority)
    }
}

impl std::fmt::Debug for OperationManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationManager")
            .field("active_count", &self.active.len())
            .field("queued_count", &self.queue.len())
            .field("max_concurrent", &self.config.max_concurrent)
            .finish_non_exhaustive()
    }
}
