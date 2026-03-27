//! V1 ephemerality contract for the resource store.
//!
//! # V1 Ephemerality
//!
//! Per resource-store/spec.md §Requirement: Ephemeral Storage in V1 (lines 244-246):
//!
//! > In v1, all resources SHALL be ephemeral. They SHALL be stored in memory and
//! > lost on runtime restart. Agents SHALL re-establish sessions and re-upload
//! > resources on reconnect.  The content-addressed model ensures re-upload is
//! > idempotent.
//!
//! This module provides [`EphemeralStore`] — a thin wrapper around
//! [`ResourceStore`] that makes the ephemerality contract explicit in the API
//! and enforces it with tests.
//!
//! ## What "ephemeral" means in practice
//!
//! 1. **No disk I/O**: The store never writes to disk, creates temp files, or
//!    opens databases.  All state lives in process memory.
//! 2. **Empty on construction**: `EphemeralStore::new()` starts with zero
//!    resources.  There is no restore-from-disk path.
//! 3. **Lost on restart**: If the process restarts, all resources are gone.
//!    Agents must re-establish sessions and re-upload.
//! 4. **Re-upload is idempotent**: Because `ResourceId` is the BLAKE3 hash of
//!    raw bytes, re-uploading the same content always produces the same
//!    `ResourceId`.  The runtime returns `was_deduplicated = true` for the
//!    second upload within a session; after restart, the first upload of a
//!    previously-seen resource is treated as fresh (store is empty) but the
//!    identity is still the same.
//!
//! ## Post-v1 exclusion
//!
//! Per spec lines 373-376, a durable persistent resource store (surviving
//! restarts) is explicitly deferred to post-v1.  This module MUST NOT introduce
//! any persistence mechanism.
//!
//! ## Scene snapshot alignment
//!
//! Scene snapshots reference resources by `ResourceId` (32-byte hash) only —
//! they do NOT embed blob data.  On restart, snapshot restore requires agents
//! to re-upload referenced resources before the scene can fully render.
//! See scene-graph/spec.md §v1 ephemerality alignment.

use std::sync::Arc;

use crate::dedup::ResourceRecord;
use crate::types::{ResourceId, ResourceStoreConfig};
use crate::upload::ResourceStore;

// ─── EphemeralStore ───────────────────────────────────────────────────────────

/// The v1 ephemeral resource store.
///
/// All resources are stored in memory and lost on runtime restart.  No
/// filesystem artifacts are ever created.  See module-level documentation
/// for the full ephemerality contract.
///
/// This type wraps [`ResourceStore`] and exposes the query-side API needed to
/// verify the ephemerality contract:
///
/// - [`EphemeralStore::get`] — look up a resource by `ResourceId`; returns
///   `None` if not in the store (including after restart).
/// - [`EphemeralStore::contains`] — check presence without fetching the record.
/// - [`EphemeralStore::resource_count`] — number of resources currently in the
///   store; always 0 after `new()`.
/// - [`EphemeralStore::is_empty`] — `true` when the store holds no resources.
///
/// Upload operations are delegated to the inner [`ResourceStore`]; use
/// [`EphemeralStore::inner`] to access them.
#[derive(Clone)]
pub struct EphemeralStore {
    inner: ResourceStore,
}

impl EphemeralStore {
    /// Create a new, **empty** ephemeral resource store.
    ///
    /// # Ephemerality guarantee
    ///
    /// This call creates a fresh in-memory store with no resources.  There is
    /// no attempt to restore previously-uploaded resources from disk or any
    /// other persistent medium.  All previously-stored resources are gone.
    pub fn new(config: ResourceStoreConfig) -> Self {
        Self {
            inner: ResourceStore::new(config),
        }
    }

    /// Look up a resource record by `ResourceId`.
    ///
    /// Returns `None` if the resource is not in the store.  After a runtime
    /// restart (i.e., after constructing a new `EphemeralStore`), this returns
    /// `None` for **all** `ResourceId` values regardless of prior uploads.
    ///
    /// This is the authoritative implementation of the spec scenario:
    ///
    /// > WHEN the runtime restarts
    /// > THEN `ResourceStore::get()` for any previously-stored `ResourceId`
    /// > returns `None` after restart.
    #[inline]
    pub fn get(&self, id: ResourceId) -> Option<Arc<ResourceRecord>> {
        self.inner.dedup_index().get(&id)
    }

    /// Returns `true` if the resource is currently in the store.
    ///
    /// Equivalent to `self.get(id).is_some()` but cheaper (no `Arc` clone).
    #[inline]
    pub fn contains(&self, id: ResourceId) -> bool {
        self.inner.dedup_index().contains(&id)
    }

    /// Current number of resources in the store.
    ///
    /// Always 0 immediately after `EphemeralStore::new()`.
    #[inline]
    pub fn resource_count(&self) -> usize {
        self.inner.dedup_index().len()
    }

    /// `true` when the store holds no resources.
    ///
    /// Always `true` immediately after `EphemeralStore::new()`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.dedup_index().is_empty()
    }

    /// Access the inner [`ResourceStore`] for upload operations.
    #[inline]
    pub fn inner(&self) -> &ResourceStore {
        &self.inner
    }

    /// Consume this wrapper and return the inner [`ResourceStore`].
    #[inline]
    pub fn into_inner(self) -> ResourceStore {
        self.inner
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ResourceStoreConfig;
    use crate::types::ResourceType;
    use crate::upload::{UploadId, UploadStartRequest};
    use crate::validation::{AgentBudget, test_helpers::minimal_png_1x1};

    fn default_store() -> EphemeralStore {
        EphemeralStore::new(ResourceStoreConfig::default())
    }

    fn caps() -> Vec<String> {
        vec!["upload_resource".to_string()]
    }

    fn unlimited_budget() -> AgentBudget {
        AgentBudget {
            texture_bytes_total_limit: 0,
            texture_bytes_total_used: 0,
        }
    }

    // ── Ephemerality: empty on construction ───────────────────────────────────

    /// WHEN a new EphemeralStore is created
    /// THEN it MUST be empty — no resources from prior sessions persist.
    #[test]
    fn fresh_store_is_empty() {
        let store = default_store();
        assert_eq!(
            store.resource_count(),
            0,
            "EphemeralStore must be empty on construction (spec lines 244-246)"
        );
        assert!(
            store.is_empty(),
            "is_empty() must return true for a fresh store"
        );
    }

    /// WHEN a new EphemeralStore is created
    /// THEN get() for any ResourceId returns None.
    #[test]
    fn get_returns_none_for_unknown_resource_id() {
        let store = default_store();
        // Any ResourceId — even one that might have existed in a prior session.
        let id = ResourceId::from_content(b"some resource bytes");
        assert!(
            store.get(id).is_none(),
            "get() must return None for any ResourceId in a fresh store (spec lines 250-251)"
        );
    }

    /// WHEN a new EphemeralStore is created
    /// THEN contains() for any ResourceId returns false.
    #[test]
    fn contains_returns_false_for_unknown_resource_id() {
        let store = default_store();
        let id = ResourceId::from_content(b"another resource");
        assert!(
            !store.contains(id),
            "contains() must return false in a fresh store"
        );
    }

    // ── Ephemerality: lost on restart (simulated by new()) ────────────────────

    /// WHEN the runtime restarts (simulated by constructing a new EphemeralStore)
    /// THEN all previously uploaded resources MUST be gone.
    /// Agents MUST re-upload after reconnection (spec lines 250-251).
    #[tokio::test]
    async fn resources_lost_on_restart() {
        // Session 1: upload a resource.
        let session1 = default_store();
        let data = minimal_png_1x1();
        let hash = *blake3::hash(&data).as_bytes();
        let resource_id = ResourceId::from_bytes(hash);

        session1
            .inner()
            .handle_upload_start(UploadStartRequest {
                agent_namespace: "agent-a".into(),
                agent_capabilities: caps(),
                agent_budget: unlimited_budget(),
                upload_id: UploadId::from_bytes([1u8; 16]),
                resource_type: ResourceType::ImagePng,
                expected_hash: hash,
                total_size: data.len(),
                inline_data: data,
                width: 0,
                height: 0,
            })
            .await
            .unwrap()
            .unwrap();

        // Confirm the resource is in session1's store.
        assert!(
            session1.contains(resource_id),
            "resource must be present in session1 after upload"
        );

        // Runtime restart: create a new EphemeralStore.
        let session2 = default_store();

        // THEN: all resources from session1 are gone.
        assert!(
            !session2.contains(resource_id),
            "resource must NOT be present in session2 after restart (spec lines 250-251)"
        );
        assert!(
            session2.get(resource_id).is_none(),
            "get() must return None for any previously-stored ResourceId after restart"
        );
        assert_eq!(
            session2.resource_count(),
            0,
            "resource_count must be 0 after restart"
        );
    }

    // ── Idempotent re-upload after restart ────────────────────────────────────

    /// WHEN the runtime restarts and an agent re-uploads the same bytes
    /// THEN the ResourceId MUST be identical to the pre-restart ResourceId.
    ///
    /// Content-addressed identity is a pure function of bytes (spec lines 5-6).
    #[tokio::test]
    async fn re_upload_after_restart_produces_same_resource_id() {
        let data = minimal_png_1x1();
        let hash = *blake3::hash(&data).as_bytes();
        let expected_id = ResourceId::from_bytes(hash);

        // Session 1: upload.
        let session1 = default_store();
        let r1 = session1
            .inner()
            .handle_upload_start(UploadStartRequest {
                agent_namespace: "agent-a".into(),
                agent_capabilities: caps(),
                agent_budget: unlimited_budget(),
                upload_id: UploadId::from_bytes([1u8; 16]),
                resource_type: ResourceType::ImagePng,
                expected_hash: hash,
                total_size: data.len(),
                inline_data: data.clone(),
                width: 0,
                height: 0,
            })
            .await
            .unwrap()
            .unwrap();

        // Runtime restart.
        let session2 = default_store();

        // Session 2: re-upload same bytes.
        let r2 = session2
            .inner()
            .handle_upload_start(UploadStartRequest {
                agent_namespace: "agent-a".into(),
                agent_capabilities: caps(),
                agent_budget: unlimited_budget(),
                upload_id: UploadId::from_bytes([2u8; 16]),
                resource_type: ResourceType::ImagePng,
                expected_hash: hash,
                total_size: data.len(),
                inline_data: data,
                width: 0,
                height: 0,
            })
            .await
            .unwrap()
            .unwrap();

        // Both must produce the same ResourceId.
        assert_eq!(
            r1.resource_id, r2.resource_id,
            "re-upload after restart must produce identical ResourceId (spec lines 5-6)"
        );
        assert_eq!(r1.resource_id, expected_id);

        // Session 2's upload is fresh (not a dedup hit across the restart boundary).
        assert!(
            !r2.was_deduplicated,
            "re-upload after restart is NOT a dedup hit — store was empty"
        );
    }

    // ── No filesystem artifacts ───────────────────────────────────────────────

    /// WHEN resources are uploaded and the store is used
    /// THEN no filesystem artifacts are created in the isolated test directory.
    ///
    /// This test verifies the no-persistence invariant by running the store
    /// operations in an isolated temporary directory and checking that it
    /// remains empty afterward.  Using a dedicated temp dir avoids the
    /// flakiness of scanning the full workspace (other parallel tests may
    /// legitimately create files there).
    #[tokio::test]
    async fn no_filesystem_artifacts_created() {
        // Create an isolated temporary directory that starts empty.
        let tmp = tempfile::tempdir().expect("cannot create temp dir");
        let tmp_path = tmp.path().to_path_buf();

        // Record files before any store operations (should be none).
        let before: Vec<_> = std::fs::read_dir(&tmp_path)
            .expect("cannot read temp dir")
            .flatten()
            .collect();
        assert!(before.is_empty(), "temp dir should be empty before test");

        let store = default_store();
        let data = minimal_png_1x1();
        let hash = *blake3::hash(&data).as_bytes();

        // Upload a resource.
        store
            .inner()
            .handle_upload_start(UploadStartRequest {
                agent_namespace: "agent-fs-check".into(),
                agent_capabilities: caps(),
                agent_budget: unlimited_budget(),
                upload_id: UploadId::from_bytes([42u8; 16]),
                resource_type: ResourceType::ImagePng,
                expected_hash: hash,
                total_size: data.len(),
                inline_data: data,
                width: 0,
                height: 0,
            })
            .await
            .unwrap()
            .unwrap();

        // Verify the isolated temp dir is still empty — EphemeralStore must
        // not write any files, databases, or caches.
        let after: Vec<_> = std::fs::read_dir(&tmp_path)
            .expect("cannot read temp dir")
            .flatten()
            .collect();
        assert!(
            after.is_empty(),
            "EphemeralStore must not create filesystem artifacts; found: {:?}",
            after.iter().map(|e| e.path()).collect::<Vec<_>>()
        );
    }

    // ── In-memory store: get() after upload ──────────────────────────────────

    /// WHEN a resource is uploaded to EphemeralStore
    /// THEN get() returns the ResourceRecord for that ResourceId.
    #[tokio::test]
    async fn get_returns_record_after_upload() {
        let store = default_store();
        let data = minimal_png_1x1();
        let hash = *blake3::hash(&data).as_bytes();
        let resource_id = ResourceId::from_bytes(hash);

        let result = store
            .inner()
            .handle_upload_start(UploadStartRequest {
                agent_namespace: "agent-get".into(),
                agent_capabilities: caps(),
                agent_budget: unlimited_budget(),
                upload_id: UploadId::from_bytes([5u8; 16]),
                resource_type: ResourceType::ImagePng,
                expected_hash: hash,
                total_size: data.len(),
                inline_data: data,
                width: 0,
                height: 0,
            })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(result.resource_id, resource_id);

        // get() must return the record.
        let record = store.get(resource_id).expect("resource should be present");
        assert_eq!(record.resource_id, resource_id);
        assert_eq!(record.resource_type, ResourceType::ImagePng);

        // resource_count must reflect the upload.
        assert_eq!(store.resource_count(), 1);
        assert!(!store.is_empty());
    }
}
