use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Instant, Duration};

#[derive(Clone)]
pub struct DedupCache {
    // Map Signature -> Timestamp (when added)
    // using u64 (e.g. millis since epoch or Instant equivalent)
    // Instant is not Send/Sync friendly for serde, but fine for memory.
    // We'll use Instant for expiration check.
    cache: Arc<DashMap<String, Instant>>,
    ttl: Duration,
}

impl DedupCache {
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            ttl: Duration::from_millis(ttl_ms),
        }
    }

    /// Returns true if signature is new (not in cache).
    /// If new, adds it to cache.
    pub fn check_and_insert(&self, signature: &str) -> bool {
        // Optimization: Try to get first to avoid write lock if exists?
        // DashMap handles concurrency well.
        // We want atomic "check if exists, if not insert".
        // insert returns the old value if it existed.
        // But we want to return false if it existed.

        if self.cache.contains_key(signature) {
            return false;
        }

        // Potential race condition here: checked, then inserted.
        // Another thread might insert in between.
        // entry() api is better.

        let entry = self.cache.entry(signature.to_string());
        match entry {
            dashmap::mapref::entry::Entry::Occupied(_) => false,
            dashmap::mapref::entry::Entry::Vacant(v) => {
                v.insert(Instant::now());
                true
            }
        }
    }

    /// Clean up old entries. Should be called periodically.
    /// This iterates the whole map, so it might be expensive if map is huge.
    /// But for 50ms requirement, we assume we process specific txs,
    /// and cleanup happens in background not blocking critical path.
    pub fn cleanup(&self) {
        // DashMap doesn't support retain well in older versions without locking shards.
        // Current dashmap supports retain.
        self.cache.retain(|_, instant| instant.elapsed() < self.ttl);
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }
}
