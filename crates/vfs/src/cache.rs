//! Directory listing cache with TTL support.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::types::{VfsEntry, VfsPath};

/// Default TTL for directory listings (30 seconds).
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(30);

/// Cached directory listing entry.
#[derive(Clone)]
struct CacheEntry {
    /// The cached entries.
    entries: Vec<VfsEntry>,
    /// When the entry was cached.
    cached_at: Instant,
    /// Time-to-live for this entry.
    ttl: Duration,
}

impl CacheEntry {
    /// Check if this cache entry has expired.
    fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > self.ttl
    }
}

/// Thread-safe directory listing cache with TTL.
pub struct DirCache {
    /// Cache storage, keyed by path URL string.
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// Default TTL for new entries.
    default_ttl: Duration,
}

impl Default for DirCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DirCache {
    /// Create a new directory cache with default TTL.
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: DEFAULT_CACHE_TTL,
        }
    }

    /// Create a directory cache with custom TTL.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl: ttl,
        }
    }

    /// Get cached directory listing if available and not expired.
    pub fn get(&self, path: &VfsPath) -> Option<Vec<VfsEntry>> {
        let key = path.to_url_string();

        let cache = self.cache.read().ok()?;
        let entry = cache.get(&key)?;

        if entry.is_expired() {
            None
        } else {
            Some(entry.entries.clone())
        }
    }

    /// Store a directory listing in the cache.
    pub fn insert(&self, path: &VfsPath, entries: Vec<VfsEntry>) {
        self.insert_with_ttl(path, entries, self.default_ttl);
    }

    /// Store a directory listing with custom TTL.
    pub fn insert_with_ttl(&self, path: &VfsPath, entries: Vec<VfsEntry>, ttl: Duration) {
        let key = path.to_url_string();

        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                key,
                CacheEntry {
                    entries,
                    cached_at: Instant::now(),
                    ttl,
                },
            );
        }
    }

    /// Invalidate a specific path.
    pub fn invalidate(&self, path: &VfsPath) {
        let key = path.to_url_string();

        if let Ok(mut cache) = self.cache.write() {
            cache.remove(&key);
        }
    }

    /// Invalidate a path and its parent (useful after file operations).
    pub fn invalidate_with_parent(&self, path: &VfsPath) {
        self.invalidate(path);
        if let Some(parent) = path.parent() {
            self.invalidate(&parent);
        }
    }

    /// Invalidate all entries matching a connection key prefix.
    pub fn invalidate_connection(&self, connection_key: &str) {
        if let Ok(mut cache) = self.cache.write() {
            cache.retain(|key, _| !key.starts_with(connection_key));
        }
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }
    }

    /// Remove expired entries (garbage collection).
    pub fn cleanup(&self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.retain(|_, entry| !entry.is_expired());
        }
    }

    /// Get the number of cached entries.
    pub fn len(&self) -> usize {
        self.cache.read().map(|c| c.len()).unwrap_or(0)
    }

    /// Check if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Clone for DirCache {
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
            default_ttl: self.default_ttl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{VfsMetadata, VfsProtocol};

    fn make_entry(name: &str) -> VfsEntry {
        VfsEntry::new(
            name,
            VfsPath::local(format!("/test/{}", name)),
            VfsMetadata::file(100),
        )
    }

    #[test]
    fn test_cache_insert_and_get() {
        let cache = DirCache::new();
        let path = VfsPath::local("/test/dir");
        let entries = vec![make_entry("file1.txt"), make_entry("file2.txt")];

        cache.insert(&path, entries.clone());

        let cached = cache.get(&path).unwrap();
        assert_eq!(cached.len(), 2);
        assert_eq!(cached[0].name, "file1.txt");
    }

    #[test]
    fn test_cache_expiration() {
        let cache = DirCache::with_ttl(Duration::from_millis(10));
        let path = VfsPath::local("/test/dir");
        let entries = vec![make_entry("file.txt")];

        cache.insert(&path, entries);

        // Should be available immediately
        assert!(cache.get(&path).is_some());

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(20));

        // Should be expired
        assert!(cache.get(&path).is_none());
    }

    #[test]
    fn test_cache_invalidate() {
        let cache = DirCache::new();
        let path = VfsPath::local("/test/dir");
        let entries = vec![make_entry("file.txt")];

        cache.insert(&path, entries);
        assert!(cache.get(&path).is_some());

        cache.invalidate(&path);
        assert!(cache.get(&path).is_none());
    }

    #[test]
    fn test_cache_invalidate_with_parent() {
        let cache = DirCache::new();
        let parent = VfsPath::local("/test");
        let child = VfsPath::local("/test/subdir");

        cache.insert(&parent, vec![make_entry("subdir")]);
        cache.insert(&child, vec![make_entry("file.txt")]);

        assert!(cache.get(&parent).is_some());
        assert!(cache.get(&child).is_some());

        cache.invalidate_with_parent(&child);

        assert!(cache.get(&parent).is_none());
        assert!(cache.get(&child).is_none());
    }

    #[test]
    fn test_cache_remote_paths() {
        let cache = DirCache::new();
        let path =
            VfsPath::remote(VfsProtocol::Sftp, "host", "/home/user").with_username("testuser");
        let entries = vec![make_entry("remote_file.txt")];

        cache.insert(&path, entries.clone());

        let cached = cache.get(&path).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].name, "remote_file.txt");
    }

    #[test]
    fn test_cache_cleanup() {
        let cache = DirCache::with_ttl(Duration::from_millis(10));
        let path1 = VfsPath::local("/test1");
        let path2 = VfsPath::local("/test2");

        cache.insert(&path1, vec![make_entry("file1.txt")]);

        // Wait for first entry to expire
        std::thread::sleep(Duration::from_millis(20));

        // Add second entry
        cache.insert(&path2, vec![make_entry("file2.txt")]);

        assert_eq!(cache.len(), 2);

        // Cleanup should remove expired entry
        cache.cleanup();

        assert_eq!(cache.len(), 1);
        assert!(cache.get(&path1).is_none());
        assert!(cache.get(&path2).is_some());
    }
}
