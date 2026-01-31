//! Worker trait and implementations for file operations.

mod conflict;
mod cross_protocol;
mod download;
mod local;
mod remote_delete;
mod upload;

use std::sync::mpsc;

use crate::types::{OperationControl, OperationProgress, OperationResult};

pub use conflict::{ConflictAction, ConflictContext};
pub use cross_protocol::{CrossProtocolDirection, CrossProtocolWorker};
pub use download::DownloadWorker;
pub use local::{LocalCopyWorker, LocalDeleteWorker};
pub use remote_delete::RemoteDeleteWorker;
pub use upload::UploadWorker;

/// Chunk size for file operations (1MB).
pub(crate) const CHUNK_SIZE: usize = 1024 * 1024;

/// Trait for operation workers.
pub trait OperationWorker: Send {
    /// Execute the operation.
    fn execute(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
    ) -> OperationResult;

    /// Execute with conflict handling support.
    fn execute_with_conflicts(
        &mut self,
        control: &OperationControl,
        progress_tx: &mpsc::Sender<OperationProgress>,
        _conflict_ctx: Option<&mut ConflictContext>,
    ) -> OperationResult {
        // Default implementation ignores conflict context
        self.execute(control, progress_tx)
    }
}
