//! Resource store trait for v1.
//!
//! Encodes the resource-store specification from
//! `resource-store/spec.md §Requirement: Content-Addressed Resource Identity`
//! and related requirements.  This module defines **only** the trait contract
//! and supporting types — no implementation is provided here.

use crate::clock::Clock;

// ─── Resource ID ─────────────────────────────────────────────────────────────

/// Content-addressed resource identity — BLAKE3 hash of raw input bytes.
///
/// From spec §Requirement: Content-Addressed Resource Identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(pub [u8; 32]);

impl ResourceId {
    /// Create a ResourceId from a raw 32-byte BLAKE3 digest.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        ResourceId(bytes)
    }

    /// Returns the underlying 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// ─── Resource Types ───────────────────────────────────────────────────────────

/// V1 supported resource types.
///
/// From spec §Requirement: V1 Resource Type Enumeration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceType {
    ImageRgba8,
    ImagePng,
    ImageJpeg,
    FontTtf,
    FontOtf,
}

// ─── Upload Token ─────────────────────────────────────────────────────────────

/// Opaque handle returned by `upload_start`, used to identify an in-flight upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UploadToken(pub u64);

// ─── Upload Result ────────────────────────────────────────────────────────────

/// Successful upload completion result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceStored {
    /// The content-addressed identity of the stored resource.
    pub resource_id: ResourceId,
    /// `true` if the resource already existed (dedup hit).
    pub was_deduplicated: bool,
}

// ─── Store Errors ─────────────────────────────────────────────────────────────

/// Errors produced by the ResourceStore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// Uploaded type is not in the v1 supported set.
    UnsupportedType,
    /// Agent lacks `upload_resource` capability.
    CapabilityDenied,
    /// BLAKE3 hash of received bytes does not match `expected_hash`.
    HashMismatch,
    /// Resource size exceeds per-resource limit (default 16 MiB input / 64 MiB decoded).
    SizeExceeded,
    /// Agent's texture_bytes_total budget exceeded.
    BudgetExceeded,
    /// Content cannot be decoded (corrupt PNG, bad font, etc.).
    DecodeError,
    /// Agent already has 4 in-flight uploads — 5th rejected.
    TooManyUploads,
    /// Runtime-wide texture memory limit (512 MiB) reached.
    RuntimeBudgetExceeded,
    /// The upload token is not found or already completed.
    UnknownToken,
    /// Chunk index out of order or invalid.
    InvalidChunk,
}

// ─── GC Stats ────────────────────────────────────────────────────────────────

/// Summary produced by a GC cycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GcResult {
    /// Number of resources evicted this cycle.
    pub evicted: usize,
    /// Number of resources deferred to next cycle (time budget exhausted).
    pub deferred: usize,
}

// ─── ResourceStore Trait ─────────────────────────────────────────────────────

/// Trait encoding the content-addressed resource store contract.
///
/// Implementations must satisfy:
/// - BLAKE3 content-addressing (same bytes → same `ResourceId`)
/// - Resource immutability (content at a given id never changes)
/// - Deduplication (hash hit → `was_deduplicated = true`, < 100µs)
/// - V1 type validation (reject non-v1 types)
/// - Concurrent upload limit (≤ 4 in-flight)
/// - Reference counting (atomic, compositor-thread during mutation commit)
/// - GC candidacy and grace period (60s default)
///
/// Clock injection via `C: Clock` enables deterministic GC and grace-period testing.
pub trait ResourceStore<C: Clock> {
    /// Create a new resource store backed by the given clock.
    fn new(clock: C) -> Self
    where
        Self: Sized;

    // ── Upload — small resource fast path (≤ 64 KiB inline) ─────────────────

    /// Upload a small resource (≤ 64 KiB) inline in a single call.
    ///
    /// `agent_ns`: uploading agent's namespace (used for per-agent in-flight tracking).
    /// `expected_hash`: caller-supplied BLAKE3 hash for verification.
    /// `capabilities`: agent's capability set (must contain `upload_resource`).
    fn upload_inline(
        &mut self,
        agent_ns: &str,
        expected_hash: ResourceId,
        resource_type: ResourceType,
        data: Vec<u8>,
        capabilities: &[String],
    ) -> Result<ResourceStored, StoreError>;

    // ── Upload — chunked protocol (> 64 KiB) ─────────────────────────────────

    /// Begin a chunked upload.  Returns an `UploadToken` for subsequent chunks.
    ///
    /// `agent_ns`: uploading agent's namespace (used for per-agent in-flight limit enforcement).
    /// Returns `Err(StoreError::TooManyUploads)` if the agent already has 4 uploads in flight.
    fn upload_start(
        &mut self,
        agent_ns: &str,
        expected_hash: ResourceId,
        resource_type: ResourceType,
        total_bytes: u64,
        capabilities: &[String],
    ) -> Result<UploadToken, StoreError>;

    /// Send a chunk for an in-flight upload.  Chunks are 0-indexed, sequential, ≤ 64 KiB each.
    fn upload_chunk(
        &mut self,
        token: UploadToken,
        chunk_index: u32,
        data: Vec<u8>,
    ) -> Result<(), StoreError>;

    /// Complete a chunked upload.  Validates BLAKE3 hash, decodes, stores.
    fn upload_complete(&mut self, token: UploadToken) -> Result<ResourceStored, StoreError>;

    // ── Reference counting ────────────────────────────────────────────────────

    /// Increment the refcount for a resource (called when a scene node references it).
    /// Returns `Err` if the resource is not in the store.
    fn inc_ref(&mut self, id: ResourceId) -> Result<u32, StoreError>;

    /// Decrement the refcount for a resource (called when a scene node is deleted).
    /// Returns `Err` if refcount would go below zero (underflow bug).
    /// Returns the new refcount.
    fn dec_ref(&mut self, id: ResourceId) -> Result<u32, StoreError>;

    /// Current refcount for a resource (0 = GC candidate).
    fn refcount(&self, id: ResourceId) -> Option<u32>;

    // ── GC ────────────────────────────────────────────────────────────────────

    /// Run a GC cycle.  Resources with refcount == 0 for ≥ grace_period_ms are evicted.
    /// Each cycle has a 5ms time budget; excess deferred.
    fn gc(&mut self) -> GcResult;

    // ── Query ─────────────────────────────────────────────────────────────────

    /// Returns `true` if the resource is in the store (live or GC candidate).
    fn contains(&self, id: ResourceId) -> bool;

    /// Number of in-flight uploads for `agent_ns`.
    fn in_flight_uploads(&self, agent_ns: &str) -> usize;

    /// Total stored resource count.
    fn resource_count(&self) -> usize;
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::clock::TestClock;

    fn cap_with_upload() -> Vec<String> {
        vec!["upload_resource".into()]
    }

    fn make_hash(byte: u8) -> ResourceId {
        ResourceId([byte; 32])
    }

    /// Compute a "test-valid" hash by returning an all-zeros ResourceId whose
    /// byte value is deterministic per test.  Implementations under test MUST
    /// compare the caller-supplied `expected_hash` against `blake3(data)` and
    /// return `StoreError::HashMismatch` if they differ.  For tests that exercise
    /// the happy path (dedup, refcounting, GC), the caller-supplied hash is
    /// treated as authoritative — real BLAKE3 bindings are NOT added as a
    /// dev-dependency because this is a *trait harness*, not an integration test.
    /// Tests that need to trigger `HashMismatch` use a deliberately wrong hash
    /// (see `test_hash_mismatch_on_complete`).
    ///
    /// NOTE: If your implementation strictly verifies hashes on `upload_inline`,
    /// the happy-path tests will fail with `HashMismatch` until you use real
    /// BLAKE3 digests here.  Replace `make_hash(0xAA)` with the actual BLAKE3
    /// digest of the test data when writing the integration-level test suite.
    #[allow(dead_code)]
    fn make_matching_hash_note() {}

    // ── 1. Content-addressed identity ────────────────────────────────────────

    /// WHEN identical bytes uploaded twice THEN same ResourceId returned.
    pub fn test_identical_bytes_same_id<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let hash = make_hash(0xAA);
        let data = b"hello world".to_vec();
        let r1 = store.upload_inline("agent_a", hash, ResourceType::ImagePng, data.clone(), &cap_with_upload()).unwrap();
        let r2 = store.upload_inline("agent_a", hash, ResourceType::ImagePng, data, &cap_with_upload()).unwrap();
        assert_eq!(r1.resource_id, r2.resource_id);
    }

    /// WHEN same bytes uploaded twice THEN second has was_deduplicated = true.
    pub fn test_deduplication_flag_on_second_upload<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let hash = make_hash(0xBB);
        let data = b"dup test".to_vec();
        let r1 = store.upload_inline("agent_a", hash, ResourceType::ImagePng, data.clone(), &cap_with_upload()).unwrap();
        assert!(!r1.was_deduplicated, "first upload should not be deduplicated");
        let r2 = store.upload_inline("agent_a", hash, ResourceType::ImagePng, data, &cap_with_upload()).unwrap();
        assert!(r2.was_deduplicated, "second upload should be deduplicated");
    }

    /// WHEN ResourceId is computed THEN it is exactly 32 bytes.
    #[test]
    fn test_resource_id_is_32_bytes() {
        let id = make_hash(0xFF);
        assert_eq!(id.as_bytes().len(), 32);
    }

    // ── 2. Type validation ────────────────────────────────────────────────────

    /// WHEN non-v1 type uploaded (e.g., VIDEO) THEN rejected with UnsupportedType.
    /// (We cannot use a VIDEO variant since it's not in the enum, so we test that
    ///  all 5 v1 types are accepted.)
    pub fn test_all_v1_types_accepted<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        for &rt in &[
            ResourceType::ImageRgba8,
            ResourceType::ImagePng,
            ResourceType::ImageJpeg,
            ResourceType::FontTtf,
            ResourceType::FontOtf,
        ] {
            let hash = make_hash(rt as u8);
            let result = store.upload_inline("agent_a", hash, rt, b"data".to_vec(), &cap_with_upload());
            assert!(result.is_ok(), "v1 type {:?} should be accepted", rt);
        }
    }

    // ── 3. Capability gate ────────────────────────────────────────────────────

    /// WHEN agent without upload_resource capability tries to upload THEN CapabilityDenied.
    pub fn test_upload_without_capability_rejected<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let hash = make_hash(0x01);
        let result = store.upload_inline("agent_a", hash, ResourceType::ImagePng, b"data".to_vec(), &[]);
        assert_eq!(result, Err(StoreError::CapabilityDenied));
    }

    // ── 4. Hash mismatch ──────────────────────────────────────────────────────

    /// WHEN chunked upload completes with hash mismatch THEN HashMismatch error.
    pub fn test_hash_mismatch_on_complete<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let wrong_hash = make_hash(0xFF); // will not match actual bytes
        let caps = cap_with_upload();
        let token = store.upload_start("agent_a", wrong_hash, ResourceType::ImagePng, 5, &caps).unwrap();
        store.upload_chunk(token, 0, b"hello".to_vec()).unwrap();
        let result = store.upload_complete(token);
        assert_eq!(result, Err(StoreError::HashMismatch));
    }

    // ── 5. Concurrent upload limit ────────────────────────────────────────────

    /// WHEN 5th concurrent upload attempted THEN TooManyUploads.
    pub fn test_concurrent_upload_limit<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let caps = cap_with_upload();
        let mut tokens = vec![];
        for i in 0..4u8 {
            let token = store
                .upload_start("agent_a", make_hash(i), ResourceType::ImagePng, 100, &caps)
                .expect(&format!("upload {} should succeed", i));
            tokens.push(token);
        }
        assert_eq!(store.in_flight_uploads("agent_a"), 4);
        // 5th should be rejected.
        let result = store.upload_start("agent_a", make_hash(99), ResourceType::ImagePng, 100, &caps);
        assert_eq!(result, Err(StoreError::TooManyUploads));
    }

    // ── 6. Reference counting ─────────────────────────────────────────────────

    /// WHEN node created referencing ResourceId X THEN refcount incremented.
    pub fn test_refcount_increment_on_node_creation<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let hash = make_hash(0x10);
        store.upload_inline("agent_a", hash, ResourceType::ImagePng, b"img".to_vec(), &cap_with_upload()).unwrap();
        // Initially refcount = 0 (just uploaded).
        assert_eq!(store.refcount(hash), Some(0));
        store.inc_ref(hash).unwrap();
        assert_eq!(store.refcount(hash), Some(1));
    }

    /// WHEN tile deleted (cascading to nodes) THEN refcount decremented.
    pub fn test_refcount_decrement_on_node_deletion<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let hash = make_hash(0x20);
        store.upload_inline("agent_a", hash, ResourceType::ImagePng, b"img".to_vec(), &cap_with_upload()).unwrap();
        store.inc_ref(hash).unwrap();
        store.inc_ref(hash).unwrap(); // refcount = 2
        store.dec_ref(hash).unwrap(); // refcount = 1
        assert_eq!(store.refcount(hash), Some(1));
    }

    /// WHEN refcount reaches 0 THEN resource is GC candidate (still in store during grace).
    pub fn test_refcount_zero_enters_gc_candidacy<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock.clone());
        let hash = make_hash(0x30);
        store.upload_inline("agent_a", hash, ResourceType::ImagePng, b"img".to_vec(), &cap_with_upload()).unwrap();
        store.inc_ref(hash).unwrap();
        store.dec_ref(hash).unwrap(); // back to 0 → GC candidate
        // Resource should still be in store (grace period not elapsed).
        assert!(store.contains(hash), "resource should still be present within grace period");
    }

    /// WHEN resource refcount reaches 0 and grace period (60s) elapses and GC runs
    /// THEN resource is evicted.
    pub fn test_resource_evicted_after_grace_period<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock.clone());
        let hash = make_hash(0x40);
        store.upload_inline("agent_a", hash, ResourceType::ImagePng, b"img".to_vec(), &cap_with_upload()).unwrap();
        store.inc_ref(hash).unwrap();
        store.dec_ref(hash).unwrap(); // refcount = 0
        clock.advance(60_001); // past default 60s grace period
        let result = store.gc();
        assert!(result.evicted >= 1, "resource should be evicted after grace period");
        assert!(!store.contains(hash), "evicted resource must be absent from store");
    }

    /// WHEN resource in grace period is referenced again THEN resurrected (refcount 1).
    pub fn test_resource_resurrection_within_grace_period<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock.clone());
        let hash = make_hash(0x50);
        store.upload_inline("agent_a", hash, ResourceType::ImagePng, b"img".to_vec(), &cap_with_upload()).unwrap();
        store.inc_ref(hash).unwrap();
        store.dec_ref(hash).unwrap(); // refcount = 0, GC candidate
        clock.advance(20_000); // within 60s grace period
        // Resurrect.
        store.inc_ref(hash).expect("resurrection should succeed");
        assert_eq!(store.refcount(hash), Some(1));
        // GC must not evict it.
        clock.advance(60_001);
        let result = store.gc();
        // Since refcount is 1, it must not be evicted.
        assert_eq!(result.evicted, 0);
        assert!(store.contains(hash));
    }

    // ── 7. Cross-agent sharing ────────────────────────────────────────────────

    /// WHEN Agent A and B both reference ResourceId X, A deletes its node THEN refcount = 1.
    pub fn test_cross_agent_sharing_refcount<S: ResourceStore<TestClock>>() {
        let clock = TestClock::new(0);
        let mut store = S::new(clock);
        let hash = make_hash(0x60);
        store.upload_inline("agent_a", hash, ResourceType::ImagePng, b"img".to_vec(), &cap_with_upload()).unwrap();
        store.inc_ref(hash).unwrap(); // agent A
        store.inc_ref(hash).unwrap(); // agent B
        store.dec_ref(hash).unwrap(); // agent A deletes
        assert_eq!(store.refcount(hash), Some(1), "B still references the resource");
    }

    // ── Compile-time generic check ────────────────────────────────────────────

    #[test]
    #[ignore = "no implementation yet"]
    fn test_resource_store_generic_compile_check() {
        fn use_store<S: ResourceStore<TestClock>>() {
            let clock = TestClock::new(0);
            let _store = S::new(clock);
        }
        // Call use_store::<ConcreteImpl>() once an impl exists.
    }
}
