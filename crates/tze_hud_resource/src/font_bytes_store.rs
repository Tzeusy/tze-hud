//! In-memory store for raw font bytes indexed by `ResourceId`.
//!
//! Only `FONT_TTF` and `FONT_OTF` resources are stored here; image resources
//! are excluded because the compositor does not need their raw bytes after
//! decode validation.
//!
//! ## Design notes
//!
//! - Bytes are stored as `Arc<[u8]>` so multiple consumers (e.g., different
//!   compositor instances in tests) can hold a reference without copying.
//! - Uses `DashMap` for the same shard-concurrent properties as `DedupIndex`.
//! - `Clone`-able via the inner `Arc` — cloning the store shares ownership of
//!   the map (and therefore the byte arcs).  This matches `ResourceStore`.
//! - GC: when font resources are evicted from the resource store, callers
//!   should call `remove` to free the bytes.  The compositor's `FontSystem`
//!   does not release fontdb entries, so eviction at the glyphon layer is out
//!   of scope for v1.

use std::sync::Arc;

use dashmap::DashMap;

use crate::types::ResourceId;

// ─── FontBytesStore ──────────────────────────────────────────────────────────

/// Thread-safe store for raw font bytes, keyed by `ResourceId`.
///
/// Shared between the upload path (writer) and any consumer that needs
/// the raw bytes (e.g., the compositor when loading fonts into glyphon).
#[derive(Clone, Debug)]
pub struct FontBytesStore {
    inner: Arc<DashMap<ResourceId, Arc<[u8]>>>,
}

impl FontBytesStore {
    /// Create a new, empty store.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    /// Insert raw font bytes for `resource_id`.
    ///
    /// If an entry already exists for this `ResourceId` (dedup race), the
    /// existing entry is kept and the new bytes are ignored — content-addressed
    /// identity guarantees the bytes are identical.
    pub fn insert(&self, resource_id: ResourceId, data: Arc<[u8]>) {
        use dashmap::mapref::entry::Entry;
        match self.inner.entry(resource_id) {
            Entry::Vacant(e) => {
                e.insert(data);
            }
            Entry::Occupied(_) => {
                // Already present (dedup race) — keep existing bytes.
            }
        }
    }

    /// Retrieve the raw bytes for `resource_id`.
    ///
    /// Returns `None` if not present (font not yet uploaded, or after eviction).
    #[inline]
    pub fn get(&self, resource_id: &ResourceId) -> Option<Arc<[u8]>> {
        self.inner.get(resource_id).map(|e| Arc::clone(e.value()))
    }

    /// Remove bytes for `resource_id` (e.g., on GC eviction).
    ///
    /// Returns the removed `Arc<[u8]>` if present.
    #[inline]
    pub fn remove(&self, resource_id: &ResourceId) -> Option<Arc<[u8]>> {
        self.inner.remove(resource_id).map(|(_, v)| v)
    }

    /// Number of font entries currently held.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` when no fonts are stored.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Default for FontBytesStore {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(n: u8) -> ResourceId {
        ResourceId::from_content(&[n])
    }

    #[test]
    fn empty_on_construction() {
        let store = FontBytesStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let store = FontBytesStore::new();
        let id = rid(1);
        let data: Arc<[u8]> = Arc::from(b"fake-font-bytes".as_ref());
        store.insert(id, Arc::clone(&data));

        let retrieved = store.get(&id).expect("should be present");
        assert_eq!(&*retrieved, b"fake-font-bytes");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn dedup_race_keeps_existing_bytes() {
        let store = FontBytesStore::new();
        let id = rid(2);
        let first: Arc<[u8]> = Arc::from(b"first".as_ref());
        let second: Arc<[u8]> = Arc::from(b"second".as_ref());

        store.insert(id, Arc::clone(&first));
        store.insert(id, Arc::clone(&second)); // should be ignored

        let retrieved = store.get(&id).expect("should be present");
        assert_eq!(&*retrieved, b"first", "existing entry must not be replaced");
    }

    #[test]
    fn get_returns_none_for_missing_id() {
        let store = FontBytesStore::new();
        assert!(store.get(&rid(0xFF)).is_none());
    }

    #[test]
    fn remove_evicts_entry() {
        let store = FontBytesStore::new();
        let id = rid(3);
        store.insert(id, Arc::from(b"bytes".as_ref()));
        assert!(!store.is_empty());

        let removed = store.remove(&id);
        assert!(removed.is_some());
        assert!(store.is_empty());
        assert!(store.get(&id).is_none());
    }

    #[test]
    fn clone_shares_state() {
        let store = FontBytesStore::new();
        let id = rid(4);
        store.insert(id, Arc::from(b"shared".as_ref()));

        // Cloning shares the inner Arc<DashMap>.
        let clone = store.clone();
        assert!(clone.get(&id).is_some(), "clone must see entries from original");

        // Insert via clone — original must also see it.
        let id2 = rid(5);
        clone.insert(id2, Arc::from(b"via-clone".as_ref()));
        assert!(store.get(&id2).is_some(), "original must see entries from clone");
    }
}
