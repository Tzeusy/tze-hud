//! Cross-agent resource sharing semantics.
//!
//! ## Spec alignment
//!
//! > Resources SHALL be global, not per-agent.  Any agent SHALL be able to
//! > reference any ResourceId if they know the hash (content-addressed identity
//! > is the capability).  There SHALL be no access control list or ownership
//! > gate on reading.  There SHALL be no "list all resources" enumeration
//! > operation to prevent resource discovery attacks.
//! >
//! > Source: resource-store/spec.md §Requirement: Cross-Agent Resource Sharing
//! > (lines 166-168), §Upload Capability Gate (lines 181-183),
//! > §Per-Agent Budget Accounting for Shared Resources (lines 151-153).
//!
//! ## Core invariants
//!
//! | Operation | Capability required? |
//! |---|---|
//! | Upload a resource | Yes — `upload_resource` |
//! | Reference (inc_ref) a ResourceId | **No** — hash knowledge is the capability |
//! | Query metadata for a known ResourceId | **No** |
//! | Enumerate all stored resources | **Not possible** — this API does not exist |
//!
//! ## Budget semantics (double-counting)
//!
//! When Agent A and Agent B each create a scene-graph node referencing the
//! same resource, each is charged the **full decoded size** against their
//! respective budgets.  This prevents coordinated multi-agent budget bypass.
//! See `budget.rs` for the implementation.
//!
//! ## Global identity (content-addressed)
//!
//! ResourceId is the BLAKE3 hash of the raw bytes, independent of the
//! uploading agent's namespace.  Two agents uploading identical bytes receive
//! the same ResourceId.  The store holds one copy of the bytes regardless of
//! how many agents reference it.
//!
//! ## No ownership
//!
//! Once a resource is stored, no agent "owns" it.  Any agent that knows the
//! ResourceId can reference it.  The refcount tracks how many scene-graph
//! nodes reference the resource — across all agents.  GC candidacy is
//! triggered only when the global refcount reaches zero (all agents have
//! deleted their nodes).

use crate::budget::BudgetRegistry;
use crate::dedup::DedupIndex;
use crate::refcount::{GcCandidateTable, RefcountError, RefcountLayer};
use crate::types::ResourceId;

// ─── RefResult ───────────────────────────────────────────────────────────────

/// Metadata returned to an agent when it creates a reference to a resource.
///
/// The agent uses this to account for the resource in its per-agent budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefResult {
    /// The resource's decoded in-memory size (bytes).  Charged against the
    /// referencing agent's budget (full size, double-counted per spec).
    pub decoded_bytes: usize,
    /// New global refcount after this operation.
    pub new_refcount: u32,
}

// ─── SharingContext ───────────────────────────────────────────────────────────

/// Cross-agent sharing context.
///
/// Combines [`RefcountLayer`] and [`BudgetRegistry`] to provide the
/// cross-agent sharing semantics defined by RFC 0011 §5:
///
/// - **Capability-free referencing**: any agent can call [`add_node_ref`] with
///   a `ResourceId` it knows — no `upload_resource` or any other capability is
///   required.
/// - **Per-agent budget accounting**: each call to [`add_node_ref`] charges the
///   full decoded size against the referencing agent's budget (double-counted).
/// - **No enumeration**: this type provides no method to list or enumerate all
///   stored resources.  Agents can only operate on `ResourceId` values they
///   already know.
///
/// [`add_node_ref`]: SharingContext::add_node_ref
#[derive(Debug)]
pub struct SharingContext {
    refcount: RefcountLayer,
    budget: BudgetRegistry,
}

impl SharingContext {
    /// Create a new sharing context backed by the given dedup index.
    ///
    /// Pass the same `DedupIndex` that the `ResourceStore` uses so that
    /// refcount operations operate on the live store.
    pub fn new(dedup: DedupIndex) -> Self {
        Self {
            refcount: RefcountLayer::new(dedup),
            budget: BudgetRegistry::new(),
        }
    }

    /// Create a sharing context sharing the given GC candidate table.
    ///
    /// Use this when the `GcRunner` must share the same candidate table
    /// instance (typical in the runtime compositor setup).
    pub fn new_with_candidates(dedup: DedupIndex, candidates: GcCandidateTable) -> Self {
        Self {
            refcount: RefcountLayer::new_with_candidates(dedup, candidates),
            budget: BudgetRegistry::new(),
        }
    }

    // ── Add a scene-graph node reference (capability-free) ────────────────────

    /// Record that `agent_ns` has created a scene-graph node referencing
    /// `resource_id`.
    ///
    /// **No capability check is performed.**  Knowing the `ResourceId` hash is
    /// the access credential (spec lines 166-168, 172-173).
    ///
    /// The referencing agent is charged the full decoded size against its
    /// per-agent budget (double-counting per spec lines 151-153).
    ///
    /// # Spec alignment
    ///
    /// > WHEN Agent A uploads an image producing ResourceId X, and Agent B
    /// > knows X, THEN Agent B MUST be able to create a node referencing
    /// > ResourceId X without additional capability.
    /// >
    /// > Source: spec lines 172-173.
    ///
    /// # Errors
    ///
    /// - `RefcountError::NotFound` — the resource is not in the store
    ///   (e.g., it was evicted by GC or was never uploaded).
    pub fn add_node_ref(
        &mut self,
        agent_ns: &str,
        resource_id: ResourceId,
        now_ms: u64,
    ) -> Result<RefResult, RefcountError> {
        // Single dedup-index lookup: increments the global refcount and
        // returns the decoded byte size in one operation, eliminating the
        // double-lookup and the associated TOCTOU window on the hot mutation
        // path.  Resurrects GC candidates automatically.
        let (decoded_bytes, new_refcount) =
            self.refcount.inc_ref_with_decoded_bytes(resource_id)?;

        // Charge the agent's budget (full decoded size per reference).
        self.budget
            .on_node_ref_added(agent_ns, resource_id, decoded_bytes);

        let _ = now_ms; // accepted for future rate-limiting use; unused for now

        Ok(RefResult {
            decoded_bytes,
            new_refcount,
        })
    }

    // ── Remove a scene-graph node reference ───────────────────────────────────

    /// Record that `agent_ns` has deleted a scene-graph node that referenced
    /// `resource_id`.
    ///
    /// The global refcount is decremented.  If it reaches zero the resource
    /// enters GC candidacy.
    ///
    /// The referencing agent's per-agent budget is reduced by the full decoded
    /// size of the resource.
    ///
    /// # Errors
    ///
    /// - `RefcountError::NotFound` — the resource is not in the store.
    /// - `RefcountError::Underflow` — refcount would go below zero (bug;
    ///   panics in debug, returns error in release).
    pub fn remove_node_ref(
        &mut self,
        agent_ns: &str,
        resource_id: ResourceId,
        now_ms: u64,
    ) -> Result<u32, RefcountError> {
        let new_refcount = self.refcount.dec_ref(resource_id, now_ms)?;
        self.budget.on_node_ref_removed(agent_ns, &resource_id);
        Ok(new_refcount)
    }

    // ── Query (capability-free) ────────────────────────────────────────────────

    /// Look up the decoded size for a resource the caller already knows.
    ///
    /// Returns `None` if the resource is not in the store.
    ///
    /// **No capability check.**  Any caller that has a `ResourceId` value may
    /// query metadata for it (spec lines 166-168).
    ///
    /// ## No enumeration
    ///
    /// This method requires the caller to supply a specific `ResourceId`.
    /// There is deliberately no method to list or enumerate all stored
    /// resources — such an operation would constitute a resource discovery
    /// attack vector (spec lines 175-177).
    pub fn query_decoded_bytes(&self, resource_id: ResourceId) -> Option<usize> {
        self.refcount
            .dedup_index()
            .get(&resource_id)
            .map(|r| r.decoded_bytes)
    }

    /// Current global refcount for a resource, or `None` if not in the store.
    pub fn refcount(&self, resource_id: ResourceId) -> Option<u32> {
        self.refcount.refcount(resource_id)
    }

    /// Returns `true` if the resource is currently in the store (live or GC
    /// candidate).
    pub fn contains(&self, resource_id: ResourceId) -> bool {
        self.refcount.contains(resource_id)
    }

    // ── Per-agent budget ──────────────────────────────────────────────────────

    /// Current total decoded bytes charged to `agent_ns`.
    ///
    /// Used by the mutation pipeline to check per-agent budget limits before
    /// committing a mutation batch (spec lines 351-353).
    pub fn agent_used_bytes(&self, agent_ns: &str) -> usize {
        self.budget.agent_used_bytes(agent_ns)
    }

    /// Verify that `agent_ns` can accommodate `additional_decoded_bytes`
    /// without breaching `agent_limit_bytes`.
    ///
    /// Returns `Err(BudgetViolation)` if the addition would exceed the limit.
    /// `agent_limit_bytes == 0` means unlimited.
    pub fn check_agent_budget(
        &self,
        agent_ns: &str,
        additional_decoded_bytes: usize,
        agent_limit_bytes: usize,
    ) -> Result<(), crate::budget::BudgetViolation> {
        self.budget
            .check_agent_limit(agent_ns, additional_decoded_bytes, agent_limit_bytes)
    }

    /// Remove all budget records for `agent_ns`.
    ///
    /// Called after agent revocation.  The resources themselves remain in
    /// the store with their current refcounts; only the budget accounting for
    /// this agent is cleared.
    pub fn remove_agent_budget(&mut self, agent_ns: &str) {
        self.budget.remove_agent(agent_ns);
    }

    // ── Internal access (for GcRunner, tests) ────────────────────────────────

    /// Access the inner `RefcountLayer` (for GC integration).
    ///
    /// Intentionally `pub(crate)` — external callers must not be able to
    /// enumerate GC candidates via `candidates().snapshot()`, which would
    /// violate the no-enumeration invariant (spec lines 175-177).
    #[allow(dead_code)] // used by GC integration path; not yet wired up end-to-end
    pub(crate) fn refcount_layer(&self) -> &RefcountLayer {
        &self.refcount
    }

    /// Access the inner `BudgetRegistry` (for diagnostics).
    ///
    /// Intentionally `pub(crate)` — exposes internal accounting state that
    /// should not leak through the public API surface.
    #[allow(dead_code)] // used by compositor diagnostics path; not yet wired up
    pub(crate) fn budget_registry(&self) -> &BudgetRegistry {
        &self.budget
    }
}

// ─── Sharing policy ───────────────────────────────────────────────────────────

/// Verify that an agent can **reference** a resource (no capability required).
///
/// Per spec lines 166-168: read/reference access has no capability gate.
/// This function always returns `Ok(())`.  It exists to make the policy
/// explicit and to serve as a call-site annotation that a gate was
/// intentionally omitted.
///
/// Contrast with [`crate::validation::check_capability`], which enforces the
/// `upload_resource` capability for write operations.
#[inline]
pub fn check_reference_policy() -> Result<(), std::convert::Infallible> {
    // No ACL. No ownership gate. Hash knowledge is the capability.
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedup::{DedupIndex, ResourceRecord};
    use crate::types::{DecodedMeta, ResourceId, ResourceType};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_resource(id: ResourceId, decoded_bytes: usize) -> ResourceRecord {
        ResourceRecord::new(
            id,
            ResourceType::ImagePng,
            &DecodedMeta {
                decoded_bytes,
                width_px: 64,
                height_px: 64,
            },
        )
    }

    /// Create a SharingContext pre-populated with one resource.
    fn context_with_resource(id: ResourceId, decoded_bytes: usize) -> SharingContext {
        let dedup = DedupIndex::new();
        dedup.insert(id, make_resource(id, decoded_bytes)).unwrap();
        SharingContext::new(dedup)
    }

    fn id(seed: &[u8]) -> ResourceId {
        ResourceId::from_content(seed)
    }

    // ── Acceptance: cross-namespace read access ───────────────────────────────

    /// WHEN Agent A uploads an image (ResourceId X is in the store) and
    /// Agent B knows X,
    /// THEN Agent B MUST be able to add a node reference without capability.
    ///
    /// Source: spec lines 172-173.
    #[test]
    fn agent_b_can_reference_resource_uploaded_by_agent_a() {
        let resource_id = id(b"shared-image");
        let decoded = 4 * 1024 * 1024; // 4 MiB

        // Store the resource (simulates Agent A's upload completing).
        let mut ctx = context_with_resource(resource_id, decoded);

        // Agent B references the resource — NO capability check.
        let result = ctx.add_node_ref("agent_b", resource_id, 0);
        assert!(
            result.is_ok(),
            "Agent B must be able to reference a resource it knows (spec lines 172-173)"
        );
        let ref_result = result.unwrap();
        assert_eq!(ref_result.decoded_bytes, decoded);
        assert_eq!(ref_result.new_refcount, 1);
    }

    /// WHEN Agent A and Agent B both reference the same ResourceId X, and
    /// Agent A deletes its node,
    /// THEN the global refcount MUST be 1 (Agent B's reference) and the
    /// resource MUST NOT be GC-eligible.
    ///
    /// Source: spec lines 142-143.
    #[test]
    fn cross_agent_sharing_refcount_after_partial_delete() {
        let resource_id = id(b"shared-refcount");
        let mut ctx = context_with_resource(resource_id, 1024);

        // Agent A and Agent B each add a node reference.
        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();
        ctx.add_node_ref("agent_b", resource_id, 0).unwrap();
        assert_eq!(ctx.refcount(resource_id), Some(2));

        // Agent A deletes its node.
        let new_rc = ctx.remove_node_ref("agent_a", resource_id, 0).unwrap();
        assert_eq!(
            new_rc, 1,
            "refcount must be 1 — Agent B still references the resource"
        );

        // Resource must NOT be in GC candidacy.
        let candidates = ctx.refcount_layer().candidates().snapshot();
        assert!(
            !candidates.iter().any(|(cid, _)| *cid == resource_id),
            "resource must not be GC-eligible while Agent B holds a reference (spec lines 142-143)"
        );
    }

    // ── Acceptance: identical bytes → identical ResourceId ────────────────────

    /// WHEN Agent A and Agent B upload the same 500 KB PNG,
    /// THEN both MUST receive the same ResourceId.
    ///
    /// Source: spec lines 11-12.
    #[test]
    fn identical_bytes_from_different_agents_produce_same_resource_id() {
        let bytes = b"shared image data";
        let id_a = ResourceId::from_content(bytes);
        let id_b = ResourceId::from_content(bytes);
        // Content-addressed identity is agent-namespace-independent.
        assert_eq!(
            id_a, id_b,
            "identical bytes from different agents must produce the same ResourceId"
        );
    }

    // ── Acceptance: per-agent double-counting ─────────────────────────────────

    /// WHEN Agent A (budget 10 MiB) and Agent B (budget 10 MiB) both reference
    /// a 4 MiB decoded texture,
    /// THEN Agent A MUST be charged 4 MiB and Agent B MUST be charged 4 MiB.
    ///
    /// Source: spec lines 157-158.
    #[test]
    fn per_agent_budget_double_counted_for_shared_resource() {
        let resource_id = id(b"4mib-texture");
        let decoded = 4 * 1024 * 1024; // 4 MiB
        let mut ctx = context_with_resource(resource_id, decoded);

        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();
        ctx.add_node_ref("agent_b", resource_id, 0).unwrap();

        assert_eq!(
            ctx.agent_used_bytes("agent_a"),
            decoded,
            "Agent A must be charged the full 4 MiB (spec lines 157-158)"
        );
        assert_eq!(
            ctx.agent_used_bytes("agent_b"),
            decoded,
            "Agent B must be charged the full 4 MiB (spec lines 157-158)"
        );
    }

    /// WHEN Agent A references a 500 KiB compressed PNG that decodes to 4 MiB
    /// RGBA8,
    /// THEN 4 MiB (decoded) MUST be charged, not 500 KiB (raw).
    ///
    /// Source: spec lines 160-162.
    #[test]
    fn decoded_size_charged_not_compressed_size() {
        let resource_id = id(b"compressed-png");
        let decoded = 4 * 1024 * 1024; // 4 MiB decoded
        let mut ctx = context_with_resource(resource_id, decoded);

        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();

        let charged = ctx.agent_used_bytes("agent_a");
        assert_eq!(
            charged, decoded,
            "decoded in-memory size must be charged, not compressed upload size"
        );
        // Ensure we would NOT accept compressed size (500 KiB) as the charge.
        assert_ne!(charged, 500 * 1024);
    }

    // ── Acceptance: no resource enumeration ───────────────────────────────────

    /// WHEN an agent has a known ResourceId, `query_decoded_bytes` returns that
    /// resource's metadata; for any other ResourceId the result is `None`.
    ///
    /// This verifies that the API only grants per-ResourceId access and does
    /// not return a collection of all stored resources.  The no-enumeration
    /// contract (spec lines 175-177) is enforced at the API design level:
    /// `SharingContext` has no `list`, `enumerate`, `iter`, `keys`, or
    /// `all_resource_ids` method.
    ///
    /// Source: spec lines 175-177.
    #[test]
    fn query_requires_known_resource_id_and_does_not_enumerate() {
        // The absence of an enumeration method is the invariant.  This test
        // demonstrates that `query_decoded_bytes` requires a specific ResourceId
        // and does not return all stored resources.
        let resource_id = id(b"only-known-resource");
        let ctx = context_with_resource(resource_id, 1024);

        // We can query a KNOWN ResourceId.
        assert!(ctx.query_decoded_bytes(resource_id).is_some());

        // For any OTHER ResourceId, the result is None — not "all resources".
        let unknown = id(b"unknown-resource");
        assert!(
            ctx.query_decoded_bytes(unknown).is_none(),
            "query of unknown ResourceId must return None, not leak other resources"
        );
    }

    // ── Acceptance: upload capability gate ───────────────────────────────────

    /// WHEN a guest agent attempts to upload a resource directly,
    /// THEN the upload MUST be rejected with RESOURCE_CAPABILITY_DENIED.
    ///
    /// Source: spec lines 187-188.
    ///
    /// Note: the capability check lives in `validation::check_capability`.
    /// This test confirms it is correctly wired: an empty capability set is
    /// rejected, and the `check_reference_policy` function always succeeds
    /// (read is cap-free while upload is gated).
    #[test]
    fn upload_requires_capability_reference_does_not() {
        use crate::validation::{CAPABILITY_UPLOAD_RESOURCE, check_capability};

        // Upload gate: guest (empty caps) is denied.
        let guest_caps: Vec<String> = vec![];
        let err = check_capability(&guest_caps).unwrap_err();
        assert_eq!(
            err,
            crate::types::ResourceError::CapabilityDenied,
            "guest agent must be denied upload (spec lines 187-188)"
        );

        // Reference gate: no capability required.
        // check_reference_policy() returns Infallible — always Ok.
        assert!(
            check_reference_policy().is_ok(),
            "referencing a ResourceId must not require any capability (spec lines 166-168)"
        );

        // Resident agent (with capability) is allowed.
        let resident_caps = vec![CAPABILITY_UPLOAD_RESOURCE.to_string()];
        assert!(check_capability(&resident_caps).is_ok());
    }

    // ── Resource not found ────────────────────────────────────────────────────

    /// WHEN an agent references a ResourceId not in the store,
    /// THEN add_node_ref returns RefcountError::NotFound.
    #[test]
    fn reference_unknown_resource_id_returns_not_found() {
        let dedup = DedupIndex::new();
        let mut ctx = SharingContext::new(dedup);
        let unknown = id(b"not-in-store");

        let result = ctx.add_node_ref("agent_a", unknown, 0);
        assert_eq!(
            result,
            Err(RefcountError::NotFound),
            "referencing an unknown ResourceId must return NotFound"
        );
    }

    // ── Budget uncharge on node delete ────────────────────────────────────────

    /// WHEN an agent deletes a node referencing a resource,
    /// THEN the per-agent budget is reduced.
    #[test]
    fn budget_uncharged_when_node_deleted() {
        let resource_id = id(b"budget-uncharge");
        let decoded = 2 * 1024 * 1024; // 2 MiB
        let mut ctx = context_with_resource(resource_id, decoded);

        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();
        assert_eq!(ctx.agent_used_bytes("agent_a"), decoded);

        ctx.remove_node_ref("agent_a", resource_id, 0).unwrap();
        assert_eq!(
            ctx.agent_used_bytes("agent_a"),
            0,
            "budget must be cleared after node deletion"
        );
    }

    // ── GC candidacy after all agents delete ─────────────────────────────────

    /// WHEN Agent A and Agent B each create a node referencing ResourceId X,
    /// and then both delete their nodes,
    /// THEN refcount MUST reach 0 and the resource MUST be GC-eligible.
    ///
    /// Source: spec lines 192-194.
    #[test]
    fn resource_enters_gc_candidacy_when_all_agents_delete() {
        let resource_id = id(b"gc-multi-agent");
        let mut ctx = context_with_resource(resource_id, 1024);

        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();
        ctx.add_node_ref("agent_b", resource_id, 0).unwrap();

        // Both agents delete their nodes.
        ctx.remove_node_ref("agent_a", resource_id, 100).unwrap();
        ctx.remove_node_ref("agent_b", resource_id, 200).unwrap();

        assert_eq!(
            ctx.refcount(resource_id),
            Some(0),
            "refcount must be 0 after all agents delete"
        );

        // Resource must now be in GC candidacy.
        let candidates = ctx.refcount_layer().candidates().snapshot();
        assert!(
            candidates.iter().any(|(cid, _)| *cid == resource_id),
            "resource must be GC-eligible when global refcount reaches 0"
        );
    }

    // ── check_reference_policy is always Ok ──────────────────────────────────

    /// The reference policy gate must always return Ok — no capability needed.
    #[test]
    fn check_reference_policy_always_succeeds() {
        // Even with no arguments, the policy always permits.
        let result = check_reference_policy();
        assert!(result.is_ok());
    }

    // ── Multiple nodes same agent same resource ───────────────────────────────

    /// WHEN Agent A creates two nodes referencing the same resource,
    /// THEN the global refcount is 2 and the agent is charged the full
    /// decoded size twice.
    #[test]
    fn multiple_nodes_same_agent_increments_refcount_and_budget_independently() {
        let resource_id = id(b"multi-node-same-agent");
        let decoded = 1024 * 1024; // 1 MiB
        let mut ctx = context_with_resource(resource_id, decoded);

        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();
        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();

        assert_eq!(ctx.refcount(resource_id), Some(2));
        assert_eq!(ctx.agent_used_bytes("agent_a"), decoded * 2);
    }

    // ── Budget check helper ───────────────────────────────────────────────────

    /// check_agent_budget returns Ok when within limit, Err when over.
    #[test]
    fn budget_check_within_and_over_limit() {
        let resource_id = id(b"budget-check");
        let decoded = 4 * 1024 * 1024; // 4 MiB
        let mut ctx = context_with_resource(resource_id, decoded);

        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();

        let ten_mib = 10 * 1024 * 1024;

        // 4 MiB used; 4 MiB more fits within 10 MiB.
        assert!(
            ctx.check_agent_budget("agent_a", 4 * 1024 * 1024, ten_mib)
                .is_ok()
        );

        // 4 MiB used; 7 MiB more would exceed 10 MiB.
        assert!(
            ctx.check_agent_budget("agent_a", 7 * 1024 * 1024, ten_mib)
                .is_err()
        );
    }

    // ── Agent revocation clears budget ────────────────────────────────────────

    /// WHEN an agent is revoked and its budget removed,
    /// THEN agent_used_bytes returns 0.
    #[test]
    fn remove_agent_budget_clears_usage() {
        let resource_id = id(b"revoke-budget");
        let mut ctx = context_with_resource(resource_id, 1024 * 1024);

        ctx.add_node_ref("agent_a", resource_id, 0).unwrap();
        assert_eq!(ctx.agent_used_bytes("agent_a"), 1024 * 1024);

        ctx.remove_agent_budget("agent_a");
        assert_eq!(ctx.agent_used_bytes("agent_a"), 0);
    }
}
