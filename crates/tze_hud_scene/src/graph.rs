//! Scene graph: the core data structure holding all tabs, tiles, nodes, leases.
//! Pure data — no GPU, no async, no I/O.

use crate::clock::{Clock, SystemClock};
use crate::types::*;
use crate::validation::ValidationError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns a `SystemClock` wrapped in `Arc<dyn Clock>`.
/// Used as the serde default for the `clock` field so that deserialized
/// graphs behave like freshly constructed ones.
fn default_clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock::new())
}

/// The root scene graph.
///
/// Time-dependent operations (lease grant, tab creation timestamps, expiry
/// checks) are routed through the injected [`Clock`].  Use
/// [`SceneGraph::new`] for production code — it installs a [`SystemClock`].
/// Use [`SceneGraph::new_with_clock`] in tests to inject a [`TestClock`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneGraph {
    /// Clock used for all `now_millis()` calls inside the graph.
    /// Skipped during serialization; restored to `SystemClock` on
    /// deserialization.
    #[serde(skip, default = "default_clock")]
    clock: Arc<dyn Clock>,
    /// All tabs, keyed by ID.
    pub tabs: HashMap<SceneId, Tab>,
    /// The currently active tab.
    pub active_tab: Option<SceneId>,
    /// All tiles, keyed by ID.
    pub tiles: HashMap<SceneId, Tile>,
    /// All nodes, keyed by ID.
    pub nodes: HashMap<SceneId, Node>,
    /// Active leases, keyed by ID.
    pub leases: HashMap<SceneId, Lease>,
    /// Hit region local state, keyed by node ID.
    pub hit_region_states: HashMap<SceneId, HitRegionLocalState>,
    /// Zone registry.
    pub zone_registry: ZoneRegistry,
    /// Sync groups, keyed by ID.
    pub sync_groups: HashMap<SceneId, SyncGroup>,
    /// Display area (the viewport dimensions).
    pub display_area: Rect,
    /// Monotonic version counter, incremented on every mutation.
    pub version: u64,
    /// Monotonically increasing sequence number assigned to each committed batch.
    ///
    /// Incremented by [`SceneGraph::next_sequence_number`] on every successful
    /// [`crate::mutation::MutationBatch`] commit. Per RFC 0001 §3.5.
    pub sequence_number: u64,
}

/// Maximum number of tabs in a scene. RFC 0001 §2.1.
pub const MAX_TABS: usize = 256;

/// Maximum number of tiles per tab. RFC 0001 §2.1.
pub const MAX_TILES_PER_TAB: usize = 1024;

/// Maximum number of nodes per tile. RFC 0001 §2.1.
pub const MAX_NODES_PER_TILE: usize = 64;

/// Maximum name length for tabs, in UTF-8 bytes. RFC 0001 §2.2.
pub const MAX_TAB_NAME_BYTES: usize = 128;

/// Maximum content size for TextMarkdownNode, in UTF-8 bytes. RFC 0001 §2.4.
pub const MAX_MARKDOWN_BYTES: usize = 65_535;

/// The z-order threshold below which agent-owned tiles must fall.
/// Tiles with z_order >= ZONE_TILE_Z_MIN are reserved for runtime-managed
/// zone tiles. RFC 0001 §2.3.
pub const ZONE_TILE_Z_MIN: u32 = 0x8000_0000;

impl SceneGraph {
    /// Create a new empty scene graph using the real system clock.
    pub fn new(width: f32, height: f32) -> Self {
        Self::new_with_clock(width, height, Arc::new(SystemClock::new()))
    }

    /// Create a new empty scene graph with an injected clock.
    ///
    /// Prefer this constructor in tests so that time-dependent behaviour
    /// (lease expiry, timestamps) is fully deterministic.
    pub fn new_with_clock(width: f32, height: f32, clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            tabs: HashMap::new(),
            active_tab: None,
            tiles: HashMap::new(),
            nodes: HashMap::new(),
            leases: HashMap::new(),
            hit_region_states: HashMap::new(),
            zone_registry: ZoneRegistry::new(),
            sync_groups: HashMap::new(),
            display_area: Rect::new(0.0, 0.0, width, height),
            version: 0,
            sequence_number: 0,
        }
    }

    // ─── Tab operations ──────────────────────────────────────────────────

    /// Create a new tab. Requires `ManageTabs` capability when `lease_id` is provided.
    ///
    /// RFC 0001 §2.2: Tab name must be non-empty, ≤ 128 UTF-8 bytes.
    /// Scene must not already have 256 tabs (MAX_TABS). RFC 0001 §2.1.
    pub fn create_tab(&mut self, name: &str, display_order: u32) -> Result<SceneId, ValidationError> {
        self.create_tab_checked(name, display_order, None)
    }

    /// Create a tab with an optional capability check against a lease.
    ///
    /// Pass `Some(lease_id)` to enforce `ManageTabs` capability. Pass `None` to skip
    /// the capability check (used by internal scene construction and tests).
    pub fn create_tab_with_lease(
        &mut self,
        name: &str,
        display_order: u32,
        lease_id: SceneId,
    ) -> Result<SceneId, ValidationError> {
        self.create_tab_checked(name, display_order, Some(lease_id))
    }

    fn create_tab_checked(
        &mut self,
        name: &str,
        display_order: u32,
        lease_id: Option<SceneId>,
    ) -> Result<SceneId, ValidationError> {
        // Capability check
        if let Some(lid) = lease_id {
            self.require_capability(lid, Capability::ManageTabs)?;
        }
        // Name validation: non-empty, ≤ 128 UTF-8 bytes (RFC 0001 §2.2)
        if name.is_empty() {
            return Err(ValidationError::InvalidField {
                field: "name".into(),
                reason: "tab name must be non-empty".into(),
            });
        }
        if name.len() > MAX_TAB_NAME_BYTES {
            return Err(ValidationError::InvalidField {
                field: "name".into(),
                reason: format!(
                    "tab name exceeds maximum {} UTF-8 bytes (got {})",
                    MAX_TAB_NAME_BYTES,
                    name.len()
                ),
            });
        }
        // Scene-level tab count limit (RFC 0001 §2.1)
        if self.tabs.len() >= MAX_TABS {
            return Err(ValidationError::BudgetExceeded {
                resource: format!("tabs (limit {})", MAX_TABS),
            });
        }
        // Check display_order uniqueness
        if self.tabs.values().any(|t| t.display_order == display_order) {
            return Err(ValidationError::DuplicateDisplayOrder { order: display_order });
        }
        let id = SceneId::new();
        let now_ms = self.clock.now_millis();
        self.tabs.insert(
            id,
            Tab {
                id,
                name: name.to_string(),
                display_order,
                created_at_ms: now_ms,
                tab_switch_on_event: None,
            },
        );
        if self.active_tab.is_none() {
            self.active_tab = Some(id);
        }
        self.version += 1;
        Ok(id)
    }

    /// Delete a tab. All tiles in the tab are also removed.
    ///
    /// RFC 0001 §2.2. Requires `ManageTabs` capability when lease is provided.
    pub fn delete_tab(&mut self, tab_id: SceneId) -> Result<(), ValidationError> {
        self.delete_tab_checked(tab_id, None)
    }

    /// Delete a tab with capability enforcement.
    pub fn delete_tab_with_lease(
        &mut self,
        tab_id: SceneId,
        lease_id: SceneId,
    ) -> Result<(), ValidationError> {
        self.delete_tab_checked(tab_id, Some(lease_id))
    }

    fn delete_tab_checked(
        &mut self,
        tab_id: SceneId,
        lease_id: Option<SceneId>,
    ) -> Result<(), ValidationError> {
        if let Some(lid) = lease_id {
            self.require_capability(lid, Capability::ManageTabs)?;
        }
        if !self.tabs.contains_key(&tab_id) {
            return Err(ValidationError::TabNotFound { id: tab_id });
        }
        // Remove all tiles in this tab (leave sync groups first to avoid dangling members)
        let tab_tiles: Vec<SceneId> = self
            .tiles
            .values()
            .filter(|t| t.tab_id == tab_id)
            .map(|t| t.id)
            .collect();
        for tile_id in tab_tiles {
            // Remove tile from its sync group before deleting the tile itself,
            // so sync_group.members does not retain a dangling tile ID.
            let _ = self.leave_sync_group(tile_id);
            self.remove_tile_and_nodes(tile_id);
        }
        self.tabs.remove(&tab_id);
        if self.active_tab == Some(tab_id) {
            // Fall back to the tab with the lowest display_order
            self.active_tab = self
                .tabs
                .values()
                .min_by_key(|t| t.display_order)
                .map(|t| t.id);
        }
        self.version += 1;
        Ok(())
    }

    /// Rename a tab. RFC 0001 §2.2. Requires `ManageTabs` capability when lease is provided.
    pub fn rename_tab(&mut self, tab_id: SceneId, new_name: &str) -> Result<(), ValidationError> {
        self.rename_tab_checked(tab_id, new_name, None)
    }

    /// Rename a tab with capability enforcement.
    pub fn rename_tab_with_lease(
        &mut self,
        tab_id: SceneId,
        new_name: &str,
        lease_id: SceneId,
    ) -> Result<(), ValidationError> {
        self.rename_tab_checked(tab_id, new_name, Some(lease_id))
    }

    fn rename_tab_checked(
        &mut self,
        tab_id: SceneId,
        new_name: &str,
        lease_id: Option<SceneId>,
    ) -> Result<(), ValidationError> {
        if let Some(lid) = lease_id {
            self.require_capability(lid, Capability::ManageTabs)?;
        }
        if new_name.is_empty() {
            return Err(ValidationError::InvalidField {
                field: "name".into(),
                reason: "tab name must be non-empty".into(),
            });
        }
        if new_name.len() > MAX_TAB_NAME_BYTES {
            return Err(ValidationError::InvalidField {
                field: "name".into(),
                reason: format!(
                    "tab name exceeds maximum {} UTF-8 bytes (got {})",
                    MAX_TAB_NAME_BYTES,
                    new_name.len()
                ),
            });
        }
        let tab = self
            .tabs
            .get_mut(&tab_id)
            .ok_or(ValidationError::TabNotFound { id: tab_id })?;
        tab.name = new_name.to_string();
        self.version += 1;
        Ok(())
    }

    /// Change the display_order of a tab. RFC 0001 §2.2.
    pub fn reorder_tab(&mut self, tab_id: SceneId, new_order: u32) -> Result<(), ValidationError> {
        self.reorder_tab_checked(tab_id, new_order, None)
    }

    /// Change the display_order of a tab with capability enforcement.
    pub fn reorder_tab_with_lease(
        &mut self,
        tab_id: SceneId,
        new_order: u32,
        lease_id: SceneId,
    ) -> Result<(), ValidationError> {
        self.reorder_tab_checked(tab_id, new_order, Some(lease_id))
    }

    fn reorder_tab_checked(
        &mut self,
        tab_id: SceneId,
        new_order: u32,
        lease_id: Option<SceneId>,
    ) -> Result<(), ValidationError> {
        if let Some(lid) = lease_id {
            self.require_capability(lid, Capability::ManageTabs)?;
        }
        if !self.tabs.contains_key(&tab_id) {
            return Err(ValidationError::TabNotFound { id: tab_id });
        }
        // display_order must be unique across tabs (excluding this tab)
        if self
            .tabs
            .values()
            .any(|t| t.id != tab_id && t.display_order == new_order)
        {
            return Err(ValidationError::DuplicateDisplayOrder { order: new_order });
        }
        let tab = self.tabs.get_mut(&tab_id).unwrap();
        tab.display_order = new_order;
        self.version += 1;
        Ok(())
    }

    pub fn switch_active_tab(&mut self, tab_id: SceneId) -> Result<(), ValidationError> {
        self.switch_active_tab_checked(tab_id, None)
    }

    /// Switch active tab with capability enforcement.
    pub fn switch_active_tab_with_lease(
        &mut self,
        tab_id: SceneId,
        lease_id: SceneId,
    ) -> Result<(), ValidationError> {
        self.switch_active_tab_checked(tab_id, Some(lease_id))
    }

    fn switch_active_tab_checked(
        &mut self,
        tab_id: SceneId,
        lease_id: Option<SceneId>,
    ) -> Result<(), ValidationError> {
        if let Some(lid) = lease_id {
            self.require_capability(lid, Capability::ManageTabs)?;
        }
        if !self.tabs.contains_key(&tab_id) {
            return Err(ValidationError::TabNotFound { id: tab_id });
        }
        self.active_tab = Some(tab_id);
        self.version += 1;
        Ok(())
    }

    // ─── Capability helpers ──────────────────────────────────────────────

    /// Check that the lease exists, is active (not expired, not suspended), and has the given capability.
    ///
    /// Returns `CapabilityMissing` if the capability is absent, `LeaseExpired`
    /// if the lease TTL has elapsed, `LeaseNotFound` if the ID is unknown,
    /// or `InvalidField` if the lease is in a non-Active state that disallows mutations.
    fn require_capability(
        &self,
        lease_id: SceneId,
        cap: Capability,
    ) -> Result<(), ValidationError> {
        let lease = self
            .leases
            .get(&lease_id)
            .ok_or(ValidationError::LeaseNotFound { id: lease_id })?;
        // Capability check before expiry: the spec says lease must be valid
        if !lease.has_capability(cap.clone()) {
            return Err(ValidationError::CapabilityMissing {
                capability: format!("{:?}", cap),
            });
        }
        // Check lease is not expired
        let now = self.clock.now_millis();
        if lease.is_expired(now) {
            return Err(ValidationError::LeaseExpired { id: lease_id });
        }
        // Check lease state allows mutations (Active only; Suspended/Disconnected block mutations)
        if !lease.is_mutations_allowed() {
            return Err(ValidationError::InvalidField {
                field: "lease_state".into(),
                reason: format!(
                    "lease {} is in {:?} state; mutations require Active state",
                    lease_id, lease.state
                ),
            });
        }
        Ok(())
    }

    /// Check that the lease is currently active (not expired, not suspended).
    fn require_active_lease(&self, lease_id: SceneId) -> Result<(), ValidationError> {
        let lease = self
            .leases
            .get(&lease_id)
            .ok_or(ValidationError::LeaseNotFound { id: lease_id })?;
        let now = self.clock.now_millis();
        if lease.is_expired(now) {
            return Err(ValidationError::LeaseExpired { id: lease_id });
        }
        if !lease.is_mutations_allowed() {
            return Err(ValidationError::InvalidField {
                field: "lease_state".into(),
                reason: format!(
                    "lease {} is in {:?} state; mutations require Active state",
                    lease_id, lease.state
                ),
            });
        }
        Ok(())
    }

    /// Configure the `tab_switch_on_event` field for an existing tab.
    ///
    /// The bare event name (e.g. `"doorbell.ring"`) triggers automatic tab
    /// activation when any agent emits a matching bare name.  Pass `None` to
    /// clear the setting.
    ///
    /// The provided bare name (when `Some`) must:
    /// - Match `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+` (validated by
    ///   [`crate::events::naming::validate_bare_name`]).
    /// - Not start with `"system."` or `"scene."` (reserved prefixes that
    ///   can never be emitted by agents and would never trigger).
    ///
    /// Spec: scene-events/spec.md §9.1–§9.4.
    pub fn set_tab_switch_on_event(
        &mut self,
        tab_id: SceneId,
        bare_name: Option<String>,
    ) -> Result<(), ValidationError> {
        if let Some(ref name) = bare_name {
            crate::events::naming::validate_bare_name(name).map_err(|e| {
                ValidationError::InvalidField {
                    field: "tab_switch_on_event".into(),
                    reason: e.to_string(),
                }
            })?;
        }
        let tab = self
            .tabs
            .get_mut(&tab_id)
            .ok_or(ValidationError::TabNotFound { id: tab_id })?;
        tab.tab_switch_on_event = bare_name;
        self.version += 1;
        Ok(())
    }

    /// Find the tab configured with `tab_switch_on_event == bare_name`.
    ///
    /// Returns `None` if no tab is configured for this bare name.
    /// Excludes any tab whose configured value starts with `"system."` (cannot
    /// match agent events — runtime enforces this at tab configuration time,
    /// but defensive check is applied here too per spec line 250).
    ///
    /// If multiple tabs share the same `tab_switch_on_event` value, the tab
    /// with the lowest `display_order` (ties broken by `created_at_ms`) is
    /// chosen to guarantee deterministic behaviour across HashMap iteration
    /// orders.
    pub fn find_tab_for_event(&self, bare_name: &str) -> Option<SceneId> {
        self.tabs
            .values()
            .filter(|tab| {
                if let Some(trigger) = &tab.tab_switch_on_event {
                    // System events must NOT trigger tab switches (spec line 250).
                    !trigger.starts_with("system.") && trigger == bare_name
                } else {
                    false
                }
            })
            .min_by_key(|tab| (tab.display_order, tab.created_at_ms))
            .map(|tab| tab.id)
    }

    // ─── Lease operations ────────────────────────────────────────────────

    /// Default maximum suspension time before a suspended lease is revoked (ms).
    /// RFC 0008 SS3.2: default 300,000 ms (5 minutes).
    pub const DEFAULT_MAX_SUSPENSION_MS: u64 = 300_000;

    /// Default grace period for disconnected leases (ms).
    /// RFC 0008 SS3.2: default 30,000 ms (30 seconds).
    pub const DEFAULT_GRACE_PERIOD_MS: u64 = 30_000;

    /// Budget soft-limit threshold (80% of hard limit).
    pub const BUDGET_SOFT_LIMIT_PCT: f64 = 0.80;

    /// Maximum leases across all agents in the entire runtime (spec §Lease Caps).
    pub const MAX_RUNTIME_LEASES: usize = 64;

    /// Default maximum leases per session (spec §Lease Caps: "max 8 default").
    ///
    /// Exposed for session-layer policy use; the scene graph enforces the hard cap
    /// (`MAX_LEASES_PER_SESSION`) when `try_grant_lease_for_session` is called.
    /// Session managers SHOULD use this constant for soft-limit enforcement.
    pub const DEFAULT_MAX_LEASES_PER_SESSION: usize = 8;

    /// Hard maximum leases per session (spec §Lease Caps: "64 hard max").
    pub const MAX_LEASES_PER_SESSION: usize = 64;

    /// Maximum tiles per lease (spec §Lease Caps).
    pub const MAX_TILES_PER_LEASE: u32 = 64;

    /// Maximum nodes per tile (spec §Lease Caps).
    pub const MAX_NODES_PER_TILE: u32 = 64;

    /// Grant a lease with a default (nil) session_id and normal priority (2).
    ///
    /// Convenience wrapper for tests and callers that do not need priority control.
    /// Production callers should use `grant_lease_with_priority` or
    /// `grant_lease_for_session` to persist the session-layer priority.
    pub fn grant_lease(
        &mut self,
        namespace: &str,
        ttl_ms: u64,
        capabilities: Vec<Capability>,
    ) -> SceneId {
        self.grant_lease_for_session(namespace, SceneId::nil(), ttl_ms, crate::lease::priority::PRIORITY_DEFAULT, capabilities)
    }

    /// Grant a lease with an explicit priority and default (nil) session_id.
    ///
    /// Persists `priority` in the `Lease` record so that the degradation ladder
    /// and arbitration engine can read it directly from the scene graph.
    ///
    /// Spec §Requirement: Priority Assignment (lease-governance/spec.md lines 49-60):
    /// the caller MUST pass the clamped priority returned by `effective_priority` /
    /// `clamp_requested_priority`; this function stores it verbatim.
    ///
    /// Panics if caps are exceeded (use `try_grant_lease_for_session` for graceful errors).
    pub fn grant_lease_with_priority(
        &mut self,
        namespace: &str,
        ttl_ms: u64,
        priority: u8,
        capabilities: Vec<Capability>,
    ) -> SceneId {
        self.grant_lease_for_session(namespace, SceneId::nil(), ttl_ms, priority, capabilities)
    }

    /// Grant a lease, enforcing runtime-wide and per-session caps.
    ///
    /// Persists `priority` in the `Lease` record so that the degradation ladder
    /// can sort by stored priority without consulting the session layer.
    ///
    /// Panics if caps are exceeded (use `try_grant_lease_for_session` for graceful errors).
    pub fn grant_lease_for_session(
        &mut self,
        namespace: &str,
        session_id: SceneId,
        ttl_ms: u64,
        priority: u8,
        capabilities: Vec<Capability>,
    ) -> SceneId {
        self.try_grant_lease_for_session(namespace, session_id, ttl_ms, priority, capabilities)
            .expect("lease grant failed cap check")
    }

    /// Try to grant a lease, returning an error if runtime or session caps are exceeded.
    ///
    /// Persists `priority` in the `Lease` record so that the degradation ladder and
    /// arbitration engine read stored priority directly from the scene graph.
    ///
    /// Spec §Requirement: Priority Assignment (lease-governance/spec.md lines 49-60):
    /// callers MUST pass the effective (clamped) priority; this function stores it verbatim.
    ///
    /// Enforces (spec §Requirement: Lease Caps):
    /// - Max 64 leases per runtime across all agents (`MAX_RUNTIME_LEASES`).
    /// - Max 64 leases per session hard cap (`MAX_LEASES_PER_SESSION`).
    ///   Session-layer policy should enforce the softer 8-lease default
    ///   (`DEFAULT_MAX_LEASES_PER_SESSION`) before calling this.
    pub fn try_grant_lease_for_session(
        &mut self,
        namespace: &str,
        session_id: SceneId,
        ttl_ms: u64,
        priority: u8,
        capabilities: Vec<Capability>,
    ) -> Result<SceneId, LeaseError> {
        // Check runtime-wide cap
        let non_terminal_count = self
            .leases
            .values()
            .filter(|l| !l.state.is_terminal())
            .count();
        if non_terminal_count >= Self::MAX_RUNTIME_LEASES {
            return Err(LeaseError::CapsExceeded(CapsError::MaxRuntimeLeasesExceeded {
                current: non_terminal_count,
                limit: Self::MAX_RUNTIME_LEASES,
            }));
        }

        // Check per-session cap (if session_id is non-nil)
        if !session_id.is_nil() {
            let session_count = self
                .leases
                .values()
                .filter(|l| l.session_id == session_id && !l.state.is_terminal())
                .count();
            if session_count >= Self::MAX_LEASES_PER_SESSION {
                return Err(LeaseError::CapsExceeded(CapsError::MaxSessionLeasesExceeded {
                    current: session_count,
                    limit: Self::MAX_LEASES_PER_SESSION,
                }));
            }
        }

        let id = SceneId::new();
        let now_ms = self.clock.now_millis();
        self.leases.insert(
            id,
            Lease {
                id,
                namespace: namespace.to_string(),
                session_id,
                state: LeaseState::Active,
                // Persist the effective priority so the degradation ladder can sort
                // by (lease_priority ASC, z_order DESC) without consulting the session layer.
                // Spec §Requirement: Priority Sort Semantics (lease-governance/spec.md lines 62-69).
                priority,
                granted_at_ms: now_ms,
                ttl_ms,
                renewal_policy: RenewalPolicy::default(),
                capabilities,
                resource_budget: ResourceBudget::default(),
                suspended_at_ms: None,
                ttl_remaining_at_suspend_ms: None,
                disconnected_at_ms: None,
                grace_period_ms: Self::DEFAULT_GRACE_PERIOD_MS,
            },
        );
        self.version += 1;
        Ok(id)
    }

    pub fn revoke_lease(&mut self, lease_id: SceneId) -> Result<(), ValidationError> {
        let namespace = {
            let lease = self
                .leases
                .get_mut(&lease_id)
                .ok_or(ValidationError::LeaseNotFound { id: lease_id })?;
            if lease.state.is_terminal() {
                return Err(ValidationError::LeaseNotFound { id: lease_id });
            }
            let ns = lease.namespace.clone();
            lease.state = LeaseState::Revoked;
            ns
        };
        // Remove all tiles associated with this lease
        let orphaned_tiles: Vec<SceneId> = self
            .tiles
            .values()
            .filter(|t| t.lease_id == lease_id)
            .map(|t| t.id)
            .collect();
        for tile_id in orphaned_tiles {
            self.remove_tile_and_nodes(tile_id);
        }
        // Spec §Requirement: Lease Revocation Clears Zone Publications
        // (lines 235–242): clear all zone pubs from this namespace on revocation.
        self.clear_zone_publications_for_namespace(&namespace);
        self.version += 1;
        Ok(())
    }

    pub fn renew_lease(&mut self, lease_id: SceneId, new_ttl_ms: u64) -> Result<(), ValidationError> {
        let lease = self
            .leases
            .get_mut(&lease_id)
            .ok_or(ValidationError::LeaseNotFound { id: lease_id })?;
        if !lease.is_active() {
            return Err(ValidationError::LeaseNotFound { id: lease_id });
        }
        lease.granted_at_ms = self.clock.now_millis();
        lease.ttl_ms = new_ttl_ms;
        self.version += 1;
        Ok(())
    }

    /// Suspend a lease (safe mode entry). Blocks mutations, preserves state.
    pub fn suspend_lease(&mut self, lease_id: &SceneId, now_ms: u64) -> Result<(), LeaseError> {
        let lease = self
            .leases
            .get_mut(lease_id)
            .ok_or(LeaseError::LeaseNotFound(*lease_id))?;
        lease.suspend(now_ms)?;
        self.version += 1;
        Ok(())
    }

    /// Resume a suspended lease (safe mode exit). Re-enables mutations.
    pub fn resume_lease(&mut self, lease_id: &SceneId, now_ms: u64) -> Result<(), LeaseError> {
        let lease = self
            .leases
            .get_mut(lease_id)
            .ok_or(LeaseError::LeaseNotFound(*lease_id))?;
        lease.resume(now_ms)?;
        self.version += 1;
        Ok(())
    }

    /// Mark a lease as disconnected (agent disconnect, enters grace period).
    ///
    /// Spec §Orphan Handling Grace Period (lines 132–145):
    /// - Lease transitions ACTIVE → ORPHANED.
    /// - TTL clock continues running.
    /// - All tiles owned by this lease receive `TileVisualHint::DisconnectionBadge`
    ///   (compositor must display the badge within 1 frame, spec line 133).
    pub fn disconnect_lease(&mut self, lease_id: &SceneId, now_ms: u64) -> Result<(), LeaseError> {
        let lease = self
            .leases
            .get_mut(lease_id)
            .ok_or(LeaseError::LeaseNotFound(*lease_id))?;
        lease.disconnect(now_ms)?;
        // Set disconnection badge on all tiles owned by this lease.
        // Compositor must render the badge within 1 frame (spec line 133).
        for tile in self.tiles.values_mut() {
            if tile.lease_id == *lease_id {
                tile.visual_hint = crate::lease::TileVisualHint::DisconnectionBadge;
            }
        }
        self.version += 1;
        Ok(())
    }

    /// Reconnect a disconnected lease (agent reconnect within grace period).
    ///
    /// Spec §Orphan Handling Grace Period (lines 139–141):
    /// ORPHANED → ACTIVE; disconnection badges cleared within 1 frame.
    pub fn reconnect_lease(&mut self, lease_id: &SceneId, now_ms: u64) -> Result<(), LeaseError> {
        let lease = self
            .leases
            .get_mut(lease_id)
            .ok_or(LeaseError::LeaseNotFound(*lease_id))?;
        lease.reconnect(now_ms)?;
        // Clear disconnection badge on all tiles owned by this lease.
        // Compositor must clear the badge within 1 frame (spec line 141).
        for tile in self.tiles.values_mut() {
            if tile.lease_id == *lease_id {
                tile.visual_hint = crate::lease::TileVisualHint::None;
            }
        }
        self.version += 1;
        Ok(())
    }

    /// Suspend all active leases (safe mode entry).
    pub fn suspend_all_leases(&mut self, now_ms: u64) {
        let active_ids: Vec<SceneId> = self
            .leases
            .values()
            .filter(|l| l.state == LeaseState::Active)
            .map(|l| l.id)
            .collect();
        for id in active_ids {
            if let Some(lease) = self.leases.get_mut(&id) {
                let _ = lease.suspend(now_ms);
            }
        }
        self.version += 1;
    }

    /// Resume all suspended leases (safe mode exit).
    pub fn resume_all_leases(&mut self, now_ms: u64) {
        let suspended_ids: Vec<SceneId> = self
            .leases
            .values()
            .filter(|l| l.state == LeaseState::Suspended)
            .map(|l| l.id)
            .collect();
        for id in suspended_ids {
            if let Some(lease) = self.leases.get_mut(&id) {
                let _ = lease.resume(now_ms);
            }
        }
        self.version += 1;
    }

    /// Expire all leases past their TTL, handle grace period expiry for
    /// disconnected leases, and handle suspension timeout.
    ///
    /// Returns detailed information about each expired/cleaned-up lease.
    pub fn expire_leases(&mut self) -> Vec<LeaseExpiry> {
        self.expire_leases_with_max_suspend(Self::DEFAULT_MAX_SUSPENSION_MS)
    }

    /// Like `expire_leases` but with a configurable max suspension time.
    pub fn expire_leases_with_max_suspend(&mut self, max_suspend_ms: u64) -> Vec<LeaseExpiry> {
        let now = self.clock.now_millis();
        let mut expiries = Vec::new();

        // Collect leases that need cleanup
        let to_process: Vec<(SceneId, LeaseState)> = self
            .leases
            .values()
            .filter_map(|l| {
                // TTL-expired active/orphaned/disconnected leases
                if (l.state == LeaseState::Active
                    || l.state == LeaseState::Orphaned
                    || l.state == LeaseState::Disconnected)
                    && l.is_expired(now)
                {
                    return Some((l.id, LeaseState::Expired));
                }
                // Grace-period-expired orphaned/disconnected leases
                if (l.state == LeaseState::Orphaned || l.state == LeaseState::Disconnected)
                    && l.check_grace_expired(now)
                {
                    return Some((l.id, LeaseState::Expired));
                }
                // Suspension-timeout leases
                if l.state == LeaseState::Suspended
                    && l.check_suspension_expired(now, max_suspend_ms)
                {
                    return Some((l.id, LeaseState::Revoked));
                }
                None
            })
            .collect();

        for (id, terminal_state) in to_process {
            // Collect the namespace before mutating so we can clear zone pubs.
            let namespace = self.leases.get(&id).map(|l| l.namespace.clone());

            // Collect tile IDs that will be removed
            let removed_tiles: Vec<SceneId> = self
                .tiles
                .values()
                .filter(|t| t.lease_id == id)
                .map(|t| t.id)
                .collect();
            for tile_id in &removed_tiles {
                self.remove_tile_and_nodes(*tile_id);
            }
            if let Some(lease) = self.leases.get_mut(&id) {
                lease.state = terminal_state;
            }

            // Spec §Requirement: Lease Revocation Clears Zone Publications
            // (lines 235–242): When a lease is REVOKED or EXPIRED, all zone
            // publications made under that lease MUST be immediately cleared.
            if terminal_state.is_terminal() {
                if let Some(ns) = namespace {
                    self.clear_zone_publications_for_namespace(&ns);
                }
            }

            expiries.push(LeaseExpiry {
                lease_id: id,
                terminal_state,
                removed_tiles,
            });
        }

        if !expiries.is_empty() {
            self.version += 1;
        }
        expiries
    }

    /// Remove all zone publications from a given agent namespace.
    ///
    /// Called on lease expiry/revocation to satisfy spec §Requirement: Lease
    /// Revocation Clears Zone Publications (lines 235–242).
    ///
    /// **Design note**: Zone publications are namespace-scoped rather than
    /// lease-scoped in v1. A namespace holds at most one non-terminal lease
    /// at a time in v1 (multi-lease atomic operations are post-v1, spec lines
    /// 325–332), so clearing by namespace is equivalent to clearing by lease.
    /// If a namespace ever has multiple concurrent leases in future versions,
    /// `ZonePublishRecord` should carry a `lease_id` field and clearing should
    /// filter by lease_id instead.
    pub fn clear_zone_publications_for_namespace(&mut self, namespace: &str) {
        for publishes in self.zone_registry.active_publishes.values_mut() {
            publishes.retain(|r| r.publisher_namespace != namespace);
        }
        // Remove empty entries for cleanliness
        self.zone_registry.active_publishes.retain(|_, v| !v.is_empty());
    }

    // ─── Budget enforcement ─────────────────────────────────────────────

    /// Get current resource usage for a lease.
    pub fn lease_resource_usage(&self, lease_id: &SceneId) -> ResourceUsage {
        let mut usage = ResourceUsage::default();
        for tile in self.tiles.values().filter(|t| t.lease_id == *lease_id) {
            usage.tiles += 1;
            // Count nodes in this tile
            let node_count = self.count_nodes_in_tile(tile);
            usage.nodes_per_tile.insert(tile.id, node_count);
            // Sum texture bytes for static image nodes in this tile
            if let Some(root_id) = tile.root_node {
                usage.texture_bytes += self.sum_texture_bytes(root_id);
            }
        }
        usage
    }

    /// Check if a mutation batch would exceed the lease's resource budget.
    ///
    /// Returns Ok(()) if within budget, or Err with the specific violation.
    pub fn check_budget(
        &self,
        lease_id: &SceneId,
        batch: &crate::mutation::MutationBatch,
    ) -> Result<(), BudgetError> {
        let lease = match self.leases.get(lease_id) {
            Some(l) => l,
            None => return Ok(()), // No lease = no budget to check
        };
        let budget = &lease.resource_budget;
        let usage = self.lease_resource_usage(lease_id);

        // Count new tiles in batch
        let new_tiles: u32 = batch
            .mutations
            .iter()
            .filter(|m| matches!(m, crate::mutation::SceneMutation::CreateTile { .. }))
            .count() as u32;

        if new_tiles > 0 {
            let projected = usage.tiles as u64 + new_tiles as u64;
            if projected > budget.max_tiles as u64 {
                return Err(BudgetError {
                    resource: "tiles".to_string(),
                    current: usage.tiles as u64,
                    limit: budget.max_tiles as u64,
                    requested: new_tiles as u64,
                });
            }
        }

        // Count new nodes per tile (AddNode / SetTileRoot)
        for mutation in &batch.mutations {
            match mutation {
                crate::mutation::SceneMutation::AddNode { tile_id, node, .. } => {
                    let current = usage.nodes_per_tile.get(tile_id).copied().unwrap_or(0);
                    let new_count = Self::count_node_tree(node);
                    let projected = current as u64 + new_count as u64;
                    if projected > budget.max_nodes_per_tile as u64 {
                        return Err(BudgetError {
                            resource: "nodes_per_tile".to_string(),
                            current: current as u64,
                            limit: budget.max_nodes_per_tile as u64,
                            requested: new_count as u64,
                        });
                    }
                }
                crate::mutation::SceneMutation::SetTileRoot { tile_id, node } => {
                    // SetTileRoot replaces the entire tree, so count new tree size
                    let new_count = Self::count_node_tree(node);
                    if new_count as u64 > budget.max_nodes_per_tile as u64 {
                        return Err(BudgetError {
                            resource: "nodes_per_tile".to_string(),
                            current: 0,
                            limit: budget.max_nodes_per_tile as u64,
                            requested: new_count as u64,
                        });
                    }
                    // Check texture bytes in new tree
                    let new_tex = Self::count_texture_bytes_in_node(node);
                    let other_tex = usage.texture_bytes
                        - self
                            .tiles
                            .get(tile_id)
                            .and_then(|t| t.root_node)
                            .map(|r| self.sum_texture_bytes(r))
                            .unwrap_or(0);
                    if other_tex + new_tex > budget.max_texture_bytes {
                        return Err(BudgetError {
                            resource: "texture_bytes".to_string(),
                            current: other_tex,
                            limit: budget.max_texture_bytes,
                            requested: new_tex,
                        });
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Check if a lease is at the soft budget warning threshold (80%).
    pub fn is_lease_budget_warning(&self, lease_id: &SceneId) -> bool {
        let lease = match self.leases.get(lease_id) {
            Some(l) => l,
            None => return false,
        };
        let usage = self.lease_resource_usage(lease_id);
        let budget = &lease.resource_budget;

        let tile_pct = usage.tiles as f64 / budget.max_tiles.max(1) as f64;
        let tex_pct = usage.texture_bytes as f64 / budget.max_texture_bytes.max(1) as f64;

        tile_pct >= Self::BUDGET_SOFT_LIMIT_PCT || tex_pct >= Self::BUDGET_SOFT_LIMIT_PCT
    }

    /// Count nodes in a tile by walking the root node tree.
    fn count_nodes_in_tile(&self, tile: &Tile) -> u32 {
        match tile.root_node {
            Some(root_id) => self.count_node_subtree(root_id),
            None => 0,
        }
    }

    pub(crate) fn count_node_subtree(&self, node_id: SceneId) -> u32 {
        match self.nodes.get(&node_id) {
            Some(node) => {
                1 + node
                    .children
                    .iter()
                    .map(|c| self.count_node_subtree(*c))
                    .sum::<u32>()
            }
            None => 0,
        }
    }

    fn sum_texture_bytes(&self, node_id: SceneId) -> u64 {
        match self.nodes.get(&node_id) {
            Some(node) => {
                let self_bytes = match &node.data {
                    NodeData::StaticImage(img) => img.decoded_bytes,
                    _ => 0,
                };
                self_bytes
                    + node
                        .children
                        .iter()
                        .map(|c| self.sum_texture_bytes(*c))
                        .sum::<u64>()
            }
            None => 0,
        }
    }

    /// Count nodes in a node tree (not yet inserted into the graph).
    fn count_node_tree(_node: &Node) -> u32 {
        // For the current node model, children are SceneIds referencing
        // other nodes. In a fresh batch submission, they would be separate
        // AddNode mutations. So we count just this node.
        1
    }

    /// Count the incoming node plus any of its children that are already in the graph.
    ///
    /// Used by `set_tile_root_impl` to validate the post-insert node count before
    /// replacing the tile root. The incoming `node` is counted as 1, and any of its
    /// `children` SceneIds that already exist in `self.nodes` are recursively counted.
    ///
    /// For a brand-new node with no pre-existing children, this returns 1 (correct).
    /// For a node whose `children` already reference persisted nodes (e.g., re-attaching
    /// an existing subtree), this returns the full subtree size, preventing the node
    /// count limit from being bypassed.
    fn count_node_tree_deep(&self, node: &Node) -> usize {
        1 + node
            .children
            .iter()
            .map(|child_id| self.count_node_subtree(*child_id) as usize)
            .sum::<usize>()
    }

    /// Count texture bytes in a node (not yet inserted into the graph).
    fn count_texture_bytes_in_node(node: &Node) -> u64 {
        match &node.data {
            NodeData::StaticImage(img) => img.decoded_bytes,
            _ => 0,
        }
    }

    // ─── Tile operations ─────────────────────────────────────────────────

    /// Create a tile. This is the unchecked form used internally for scene construction.
    ///
    /// For agent-facing operations use [`create_tile_checked`] which enforces:
    /// - Lease active + `CreateTiles` + `ModifyOwnTiles` capabilities
    /// - Per-tab tile count limit (1024)
    /// - Bounds positive-size and within-display-area
    /// - z_order < ZONE_TILE_Z_MIN
    pub fn create_tile(
        &mut self,
        tab_id: SceneId,
        namespace: &str,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
    ) -> Result<SceneId, ValidationError> {
        self.create_tile_impl(tab_id, namespace, lease_id, bounds, z_order, false)
    }

    /// Create a tile with full spec-compliant validation including capability checks.
    ///
    /// RFC 0001 §2.3, §3.1, §3.3: requires active lease, `create_tiles`, and
    /// `modify_own_tiles` capabilities. Enforces per-tab tile limit, bounds invariants,
    /// and z_order zone-band reservation.
    pub fn create_tile_checked(
        &mut self,
        tab_id: SceneId,
        namespace: &str,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
    ) -> Result<SceneId, ValidationError> {
        self.create_tile_impl(tab_id, namespace, lease_id, bounds, z_order, true)
    }

    fn create_tile_impl(
        &mut self,
        tab_id: SceneId,
        namespace: &str,
        lease_id: SceneId,
        bounds: Rect,
        z_order: u32,
        enforce_capabilities: bool,
    ) -> Result<SceneId, ValidationError> {
        // Validate tab exists
        if !self.tabs.contains_key(&tab_id) {
            return Err(ValidationError::TabNotFound { id: tab_id });
        }

        if enforce_capabilities {
            // Lease must be active and have create_tiles + modify_own_tiles
            self.require_active_lease(lease_id)?;
            self.require_capability(lease_id, Capability::CreateTiles)?;
            self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

            // Namespace isolation: the caller's namespace must match the lease's namespace.
            // This prevents an agent from creating tiles in another agent's namespace
            // using their own (valid) lease. RFC 0001 §1.2.
            let lease_namespace = self
                .leases
                .get(&lease_id)
                .map(|l| l.namespace.as_str())
                .unwrap_or("");
            if namespace != lease_namespace {
                return Err(ValidationError::NamespaceMismatch {
                    tile_id: lease_id, // use lease_id as context; tile not created yet
                    tile_namespace: lease_namespace.to_string(),
                    agent_namespace: namespace.to_string(),
                });
            }
        } else {
            // Validate lease exists at minimum
            if !self.leases.contains_key(&lease_id) {
                return Err(ValidationError::LeaseNotFound { id: lease_id });
            }
        }

        // Per-tab tile count limit (RFC 0001 §2.1: max 1024 tiles per tab)
        let tiles_in_tab = self.tiles.values().filter(|t| t.tab_id == tab_id).count();
        if tiles_in_tab >= MAX_TILES_PER_TAB {
            return Err(ValidationError::BudgetExceeded {
                resource: format!("tiles_per_tab (limit {})", MAX_TILES_PER_TAB),
            });
        }

        // Bounds: width and height must be > 0 (RFC 0001 §2.3)
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds width ({}) and height ({}) must be > 0.0",
                    bounds.width, bounds.height
                ),
            });
        }

        // Bounds must be fully within the tab display area (RFC 0001 §2.3)
        if !bounds.is_within(&self.display_area) {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds ({},{} {}×{}) are not fully within display area ({},{} {}×{})",
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                    self.display_area.x,
                    self.display_area.y,
                    self.display_area.width,
                    self.display_area.height,
                ),
            });
        }

        // z_order must be < ZONE_TILE_Z_MIN for agent-owned tiles (RFC 0001 §2.3)
        if z_order >= ZONE_TILE_Z_MIN {
            return Err(ValidationError::InvalidField {
                field: "z_order".into(),
                reason: format!(
                    "z_order 0x{:08X} is >= ZONE_TILE_Z_MIN (0x{:08X}); reserved for runtime zone tiles",
                    z_order, ZONE_TILE_Z_MIN
                ),
            });
        }

        let id = SceneId::new();
        self.tiles.insert(
            id,
            Tile {
                id,
                tab_id,
                namespace: namespace.to_string(),
                lease_id,
                bounds,
                z_order,
                opacity: 1.0,
                input_mode: InputMode::Capture,
                sync_group: None,
                present_at: None,
                expires_at: None,
                resource_budget: ResourceBudget::default(),
                root_node: None,
                visual_hint: crate::lease::TileVisualHint::None,
            },
        );
        self.version += 1;
        Ok(id)
    }

    /// Update the bounds of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles` capability.
    /// Bounds must be positive and within the display area.
    pub fn update_tile_bounds(
        &mut self,
        tile_id: SceneId,
        bounds: Rect,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds width ({}) and height ({}) must be > 0.0",
                    bounds.width, bounds.height
                ),
            });
        }
        if !bounds.is_within(&self.display_area) {
            return Err(ValidationError::BoundsOutOfRange {
                reason: format!(
                    "tile bounds ({},{} {}×{}) are not fully within display area",
                    bounds.x, bounds.y, bounds.width, bounds.height
                ),
            });
        }

        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.bounds = bounds;
        self.version += 1;
        Ok(())
    }

    /// Update the z-order of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`.
    /// z_order must be < ZONE_TILE_Z_MIN.
    pub fn update_tile_z_order(
        &mut self,
        tile_id: SceneId,
        z_order: u32,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        if z_order >= ZONE_TILE_Z_MIN {
            return Err(ValidationError::InvalidField {
                field: "z_order".into(),
                reason: format!(
                    "z_order 0x{:08X} is >= ZONE_TILE_Z_MIN (0x{:08X}); reserved for runtime zone tiles",
                    z_order, ZONE_TILE_Z_MIN
                ),
            });
        }

        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.z_order = z_order;
        self.version += 1;
        Ok(())
    }

    /// Update the opacity of a tile.
    ///
    /// RFC 0001 §2.3: opacity must be in [0.0, 1.0]. Requires active lease + `ModifyOwnTiles`.
    pub fn update_tile_opacity(
        &mut self,
        tile_id: SceneId,
        opacity: f32,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        if !(0.0..=1.0).contains(&opacity) {
            return Err(ValidationError::InvalidField {
                field: "opacity".into(),
                reason: format!(
                    "opacity {} is not in [0.0, 1.0]",
                    opacity
                ),
            });
        }

        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.opacity = opacity;
        self.version += 1;
        Ok(())
    }

    /// Update the input mode of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`.
    pub fn update_tile_input_mode(
        &mut self,
        tile_id: SceneId,
        input_mode: InputMode,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.input_mode = input_mode;
        self.version += 1;
        Ok(())
    }

    /// Update the expiry timestamp of a tile.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`.
    pub fn update_tile_expiry(
        &mut self,
        tile_id: SceneId,
        expires_at: Option<u64>,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.expires_at = expires_at;
        self.version += 1;
        Ok(())
    }

    /// Delete a tile and all its nodes.
    ///
    /// RFC 0001 §2.3: requires active lease + `ModifyOwnTiles`. Namespace isolation enforced.
    pub fn delete_tile(
        &mut self,
        tile_id: SceneId,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        let lease_id = self.get_tile_lease_checked(tile_id, agent_namespace)?;
        self.require_active_lease(lease_id)?;
        self.require_capability(lease_id, Capability::ModifyOwnTiles)?;

        // Leave sync group before removing the tile to avoid dangling member entries.
        let _ = self.leave_sync_group(tile_id);
        self.remove_tile_and_nodes(tile_id);
        self.version += 1;
        Ok(())
    }

    /// Get the lease ID for a tile, enforcing namespace isolation.
    ///
    /// Returns `NamespaceMismatch` if the tile belongs to a different namespace.
    /// Returns `TileNotFound` if the tile does not exist.
    fn get_tile_lease_checked(
        &self,
        tile_id: SceneId,
        agent_namespace: &str,
    ) -> Result<SceneId, ValidationError> {
        let tile = self
            .tiles
            .get(&tile_id)
            .ok_or(ValidationError::TileNotFound { id: tile_id })?;
        if tile.namespace != agent_namespace {
            return Err(ValidationError::NamespaceMismatch {
                tile_id,
                tile_namespace: tile.namespace.clone(),
                agent_namespace: agent_namespace.to_string(),
            });
        }
        Ok(tile.lease_id)
    }

    pub fn set_tile_root(&mut self, tile_id: SceneId, node: Node) -> Result<(), ValidationError> {
        self.set_tile_root_impl(tile_id, node, None)
    }

    /// Set tile root with full capability and node-count enforcement.
    pub fn set_tile_root_checked(
        &mut self,
        tile_id: SceneId,
        node: Node,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        self.set_tile_root_impl(tile_id, node, Some(agent_namespace))
    }

    fn set_tile_root_impl(
        &mut self,
        tile_id: SceneId,
        node: Node,
        agent_namespace: Option<&str>,
    ) -> Result<(), ValidationError> {
        if let Some(ns) = agent_namespace {
            let lease_id = self.get_tile_lease_checked(tile_id, ns)?;
            self.require_active_lease(lease_id)?;
            self.require_capability(lease_id, Capability::ModifyOwnTiles)?;
        }

        // Check for duplicate node ID (scene-globally unique per RFC 0001 §2.1)
        if self.nodes.contains_key(&node.id) {
            return Err(ValidationError::DuplicateId { id: node.id });
        }

        // Validate node data constraints (e.g. TextMarkdownNode content size limit)
        if let Some(err) = validate_text_markdown_node_data(&node.data) {
            return Err(err);
        }

        // Node count limit: SetTileRoot replaces the whole tree.
        // Count nodes in the incoming tree (simple count; children are flat in our model).
        let incoming_count = self.count_node_tree_deep(&node);
        if incoming_count > MAX_NODES_PER_TILE {
            return Err(ValidationError::NodeCountExceeded {
                tile_id,
                current: incoming_count,
                limit: MAX_NODES_PER_TILE,
            });
        }

        // Get old root first, then release the borrow
        let old_root = {
            let tile = self
                .tiles
                .get(&tile_id)
                .ok_or(ValidationError::TileNotFound { id: tile_id })?;
            tile.root_node
        };

        // Remove old root and its subtree if present
        if let Some(old_root_id) = old_root {
            self.remove_node_tree(old_root_id);
        }

        let node_id = node.id;

        // Initialize hit region local state if applicable
        if let NodeData::HitRegion(_) = &node.data {
            self.hit_region_states
                .insert(node_id, HitRegionLocalState::new(node_id));
        }

        // Insert the node and all children recursively
        self.insert_node_tree(&node);

        // Set the new root on the tile
        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.root_node = Some(node_id);

        self.version += 1;
        Ok(())
    }

    pub fn add_node_to_tile(
        &mut self,
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
    ) -> Result<(), ValidationError> {
        self.add_node_to_tile_impl(tile_id, parent_id, node, None)
    }

    /// Add a node to a tile with full spec-compliant validation.
    pub fn add_node_to_tile_checked(
        &mut self,
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        self.add_node_to_tile_impl(tile_id, parent_id, node, Some(agent_namespace))
    }

    fn add_node_to_tile_impl(
        &mut self,
        tile_id: SceneId,
        parent_id: Option<SceneId>,
        node: Node,
        agent_namespace: Option<&str>,
    ) -> Result<(), ValidationError> {
        if let Some(ns) = agent_namespace {
            let lease_id = self.get_tile_lease_checked(tile_id, ns)?;
            self.require_active_lease(lease_id)?;
            self.require_capability(lease_id, Capability::ModifyOwnTiles)?;
        } else if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }

        // Check for duplicate node ID (RFC 0001 §2.1: NodeIds must be scene-globally unique)
        if self.nodes.contains_key(&node.id) {
            return Err(ValidationError::DuplicateId { id: node.id });
        }

        // Validate node data constraints (e.g. TextMarkdownNode content size limit)
        if let Some(err) = validate_text_markdown_node_data(&node.data) {
            return Err(err);
        }

        // Enforce per-tile node count limit (RFC 0001 §2.1: max 64 nodes)
        let current_count = self.count_nodes_in_tile(
            self.tiles.get(&tile_id).unwrap()
        ) as usize;
        if current_count >= MAX_NODES_PER_TILE {
            return Err(ValidationError::NodeCountExceeded {
                tile_id,
                current: current_count,
                limit: MAX_NODES_PER_TILE,
            });
        }

        let node_id = node.id;

        // If parent specified, add as child
        if let Some(pid) = parent_id {
            let parent = self
                .nodes
                .get_mut(&pid)
                .ok_or(ValidationError::NodeNotFound { id: pid })?;
            parent.children.push(node_id);
        } else {
            // Set as root if no root exists
            let tile = self.tiles.get_mut(&tile_id).unwrap();
            if tile.root_node.is_none() {
                tile.root_node = Some(node_id);
            }
        }

        // Track hit region state
        if let NodeData::HitRegion(_) = &node.data {
            self.hit_region_states
                .insert(node_id, HitRegionLocalState::new(node_id));
        }

        self.insert_node_tree(&node);
        self.version += 1;
        Ok(())
    }

    // ─── Sync group operations ───────────────────────────────────────────

    /// Maximum sync groups per agent namespace (RFC 0003 §2.5).
    pub const MAX_SYNC_GROUPS_PER_NAMESPACE: usize = 16;

    /// Maximum tiles per sync group (RFC 0003 §2.5).
    pub const MAX_MEMBERS_PER_SYNC_GROUP: usize = 64;

    /// Create a new sync group. Returns the new sync group ID.
    pub fn create_sync_group(
        &mut self,
        name: Option<String>,
        owner_namespace: &str,
        commit_policy: SyncCommitPolicy,
        max_deferrals: u32,
    ) -> Result<SceneId, ValidationError> {
        // Enforce per-namespace limit (RFC 0003 §2.5)
        let existing_count = self
            .sync_groups
            .values()
            .filter(|sg| sg.owner_namespace == owner_namespace)
            .count();
        if existing_count >= Self::MAX_SYNC_GROUPS_PER_NAMESPACE {
            return Err(ValidationError::SyncGroupLimitExceeded {
                limit: Self::MAX_SYNC_GROUPS_PER_NAMESPACE,
            });
        }

        let id = SceneId::new();
        let created_at_us = now_micros();
        self.sync_groups.insert(
            id,
            SyncGroup::new(
                id,
                name,
                owner_namespace.to_string(),
                commit_policy,
                max_deferrals,
                created_at_us,
            ),
        );
        self.version += 1;
        Ok(id)
    }

    /// Delete a sync group. All member tiles are automatically released.
    pub fn delete_sync_group(&mut self, group_id: SceneId) -> Result<(), ValidationError> {
        if let Some(group) = self.sync_groups.remove(&group_id) {
            // Release only the tiles that are members of this group.
            // Iterating the member set is O(k) where k = member count, not O(n tiles).
            for tile_id in group.members {
                if let Some(tile) = self.tiles.get_mut(&tile_id) {
                    tile.sync_group = None;
                }
            }
            self.version += 1;
            Ok(())
        } else {
            Err(ValidationError::SyncGroupNotFound { id: group_id })
        }
    }

    /// Add a tile to a sync group.
    ///
    /// A tile may belong to at most one sync group (RFC 0003 §2.3). Joining
    /// replaces any previous group membership.
    pub fn join_sync_group(
        &mut self,
        tile_id: SceneId,
        group_id: SceneId,
    ) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }
        if !self.sync_groups.contains_key(&group_id) {
            return Err(ValidationError::SyncGroupNotFound { id: group_id });
        }

        // Enforce member limit
        let member_count = self
            .sync_groups
            .get(&group_id)
            .map(|sg| sg.members.len())
            .unwrap_or(0);
        // Only enforce if tile is not already in this group
        let already_member = self
            .sync_groups
            .get(&group_id)
            .map(|sg| sg.members.contains(&tile_id))
            .unwrap_or(false);
        if !already_member && member_count >= Self::MAX_MEMBERS_PER_SYNC_GROUP {
            return Err(ValidationError::SyncGroupMemberLimitExceeded {
                limit: Self::MAX_MEMBERS_PER_SYNC_GROUP,
            });
        }

        // If tile is currently in a different group, remove it from that group first
        let current_group = self.tiles.get(&tile_id).and_then(|t| t.sync_group);
        if let Some(old_group_id) = current_group
            && old_group_id != group_id
            && let Some(old_group) = self.sync_groups.get_mut(&old_group_id)
        {
            old_group.members.remove(&tile_id);
        }

        // Update tile's sync_group reference
        let tile = self.tiles.get_mut(&tile_id).unwrap();
        tile.sync_group = Some(group_id);

        // Add to the group's member set
        self.sync_groups
            .get_mut(&group_id)
            .unwrap()
            .members
            .insert(tile_id);

        self.version += 1;
        Ok(())
    }

    /// Remove a tile from its sync group.
    ///
    /// Removes the tile from whatever group it currently belongs to.
    /// If the tile is not in any group, this is a no-op (returns Ok).
    /// If the group becomes empty after the last member leaves it is **not**
    /// automatically destroyed — destruction is explicit (RFC 0003 §2.3).
    pub fn leave_sync_group(&mut self, tile_id: SceneId) -> Result<(), ValidationError> {
        if !self.tiles.contains_key(&tile_id) {
            return Err(ValidationError::TileNotFound { id: tile_id });
        }
        let current_group = self.tiles.get(&tile_id).and_then(|t| t.sync_group);
        if let Some(group_id) = current_group {
            if let Some(group) = self.sync_groups.get_mut(&group_id) {
                group.members.remove(&tile_id);
            }
            let tile = self.tiles.get_mut(&tile_id).unwrap();
            tile.sync_group = None;
        }
        self.version += 1;
        Ok(())
    }

    /// Evaluate a sync group's commit policy for a given set of tiles that
    /// have pending mutations this frame.
    ///
    /// Returns a `SyncGroupCommitDecision` describing whether to commit,
    /// defer, or force-commit the group.
    ///
    /// This is called by the compositor at Stage 4 (Scene Commit).
    ///
    /// # Correctness invariant
    ///
    /// `deferral_count` MUST only increment when at least one member has a
    /// pending mutation AND at least one member is absent. When the group is
    /// idle (zero pending mutations), the counter MUST NOT change.
    /// Spec: timing-model/spec.md lines 159, 167–169.
    pub fn evaluate_sync_group_commit(
        &mut self,
        group_id: SceneId,
        tiles_with_pending: &std::collections::BTreeSet<SceneId>,
    ) -> Result<SyncGroupCommitDecision, ValidationError> {
        use crate::timing::sync_commit::{evaluate_commit, apply_decision, CommitDecision};

        let group = self
            .sync_groups
            .get(&group_id)
            .ok_or(ValidationError::SyncGroupNotFound { id: group_id })?;

        let decision = evaluate_commit(group, tiles_with_pending);

        // Translate CommitDecision → SyncGroupCommitDecision and apply state
        // changes to the group.
        let result = match &decision {
            CommitDecision::Commit { tiles } => {
                SyncGroupCommitDecision::Commit { tiles: tiles.clone() }
            }
            CommitDecision::Defer => SyncGroupCommitDecision::Defer,
            CommitDecision::ForceCommit { committed_tiles, .. } => {
                SyncGroupCommitDecision::ForceCommit { tiles: committed_tiles.clone() }
            }
        };

        // Apply state mutation (deferral_count update) to the group.
        let group_mut = self.sync_groups.get_mut(&group_id).unwrap();
        apply_decision(group_mut, &decision);

        Ok(result)
    }

    /// Add a tile to a sync group with an ownership check.
    ///
    /// The `agent_namespace` must match both the tile's namespace and the
    /// sync group's `owner_namespace`. This enforces the spec rule that an
    /// agent MUST NOT place another agent's tiles into a sync group.
    ///
    /// Spec: timing-model/spec.md lines 188–189.
    pub fn join_sync_group_checked(
        &mut self,
        tile_id: SceneId,
        group_id: SceneId,
        agent_namespace: &str,
    ) -> Result<(), ValidationError> {
        use crate::timing::sync_group::check_sync_group_ownership;

        let tile = self.tiles.get(&tile_id).ok_or(ValidationError::TileNotFound { id: tile_id })?;
        let group = self.sync_groups.get(&group_id).ok_or(ValidationError::SyncGroupNotFound { id: group_id })?;

        check_sync_group_ownership(agent_namespace, &tile.namespace, &group.owner_namespace)
            .map_err(|reason| ValidationError::SyncGroupOwnershipViolation { reason })?;

        self.join_sync_group(tile_id, group_id)
    }

    /// Return the number of sync groups in the scene.
    pub fn sync_group_count(&self) -> usize {
        self.sync_groups.len()
    }

    // ─── Node tree helpers ───────────────────────────────────────────────

    fn insert_node_tree(&mut self, node: &Node) {
        // Insert children first (depth-first)
        for child_id in &node.children {
            // Children should already be in the node or will be added separately
            // For the vertical slice, nodes are self-contained with their children
            let _ = child_id;
        }
        self.nodes.insert(node.id, node.clone());
    }

    pub(crate) fn remove_node_tree(&mut self, node_id: SceneId) {
        if let Some(node) = self.nodes.remove(&node_id) {
            for child_id in &node.children {
                self.remove_node_tree(*child_id);
            }
        }
        self.hit_region_states.remove(&node_id);
    }

    pub(crate) fn remove_tile_and_nodes(&mut self, tile_id: SceneId) {
        if let Some(tile) = self.tiles.remove(&tile_id)
            && let Some(root_id) = tile.root_node
        {
            self.remove_node_tree(root_id);
        }
    }

    // ─── Queries ─────────────────────────────────────────────────────────

    /// Get all tiles on the active tab, sorted by z_order (back to front).
    pub fn visible_tiles(&self) -> Vec<&Tile> {
        let active = match self.active_tab {
            Some(id) => id,
            None => return vec![],
        };
        let mut tiles: Vec<&Tile> = self
            .tiles
            .values()
            .filter(|t| t.tab_id == active)
            .collect();
        tiles.sort_by_key(|t| t.z_order);
        tiles
    }

    /// Map a 2D display-coordinate point to the deepest interactive element.
    ///
    /// Traversal order (per scene-graph/spec.md §Requirement: Hit-Testing Contract,
    /// RFC 0001 §5.1-5.2, and input-model/spec.md lines 263-274):
    ///
    /// 1. **Chrome layer first** — tiles whose lease has priority 0 are checked
    ///    before any content-layer tile, regardless of z-order.  The first
    ///    non-passthrough chrome tile whose bounds contain the point wins and
    ///    returns [`HitResult::Chrome`].
    /// 2. **Content layer tiles by z-order descending** — remaining (non-chrome)
    ///    tiles sorted highest z-order first.  Passthrough tiles are skipped.
    /// 3. **Within each tile, reverse tree order** — node children visited
    ///    last-first (last sibling = front-most); depth-first.  Only
    ///    [`NodeData::HitRegion`] nodes with `accepts_pointer = true` qualify.
    ///
    /// # Return value
    /// - [`HitResult::Chrome`]   — chrome-layer tile/node absorbed the point.
    /// - [`HitResult::NodeHit`]  — a `HitRegionNode` within a content tile matched.
    /// - [`HitResult::TileHit`]  — the tile absorbed the point but no node matched.
    /// - [`HitResult::Passthrough`] — only passthrough tiles at this coordinate.
    ///
    /// Returns [`HitResult::Passthrough`] when no tile covers the point.
    ///
    /// # Performance
    /// Pure geometry — no GPU involvement.  Target: < 100 µs for 50 tiles
    /// (scene-graph/spec.md line 267, RFC 0001 §10).
    pub fn hit_test(&self, x: f32, y: f32) -> HitResult {
        let Some(active) = self.active_tab else {
            return HitResult::Passthrough;
        };

        // Gather all tiles on the active tab that cover the point.
        // Partition into chrome (priority-0 lease) and content.
        let mut chrome_tiles: Vec<&Tile> = Vec::new();
        let mut content_tiles: Vec<&Tile> = Vec::new();

        for tile in self.tiles.values().filter(|t| t.tab_id == active) {
            if !tile.bounds.contains_point(x, y) {
                continue;
            }
            let is_chrome = self
                .leases
                .get(&tile.lease_id)
                .map(|l| l.priority == 0)
                .unwrap_or(false);
            if is_chrome {
                chrome_tiles.push(tile);
            } else {
                content_tiles.push(tile);
            }
        }

        // ── Phase 1: Chrome layer ────────────────────────────────────────
        // Sort chrome tiles highest z-order first; passthrough chrome tiles
        // do NOT block (they are skipped), but a non-passthrough chrome tile
        // wins immediately.
        chrome_tiles.sort_by(|a, b| b.z_order.cmp(&a.z_order));
        for tile in &chrome_tiles {
            if tile.input_mode == InputMode::Passthrough {
                continue;
            }
            // Chrome tile absorbs the hit.  If it has a HitRegionNode, report
            // its node_id as the element_id for richer routing; otherwise use
            // the tile id.
            let local_x = x - tile.bounds.x;
            let local_y = y - tile.bounds.y;
            let element_id = tile
                .root_node
                .and_then(|root| self.hit_test_node(root, local_x, local_y))
                .unwrap_or(tile.id);
            return HitResult::Chrome { element_id };
        }

        // ── Phase 2: Content layer tiles (z-order descending) ────────────
        content_tiles.sort_by(|a, b| b.z_order.cmp(&a.z_order));
        for tile in &content_tiles {
            if tile.input_mode == InputMode::Passthrough {
                continue; // Skip passthrough tiles per spec.
            }
            let local_x = x - tile.bounds.x;
            let local_y = y - tile.bounds.y;

            // ── Phase 3: Within the tile — reverse tree order ────────────
            if let Some(root_id) = tile.root_node {
                if let Some(node_id) = self.hit_test_node(root_id, local_x, local_y) {
                    // Retrieve interaction_id from the node (it must be HitRegionNode).
                    let interaction_id = self
                        .nodes
                        .get(&node_id)
                        .and_then(|n| {
                            if let NodeData::HitRegion(hr) = &n.data {
                                Some(hr.interaction_id.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    return HitResult::NodeHit { tile_id: tile.id, node_id, interaction_id };
                }
            }

            // Tile absorbed the point but no HitRegionNode matched.
            return HitResult::TileHit { tile_id: tile.id };
        }

        // Only passthrough tiles covered the point, or no tiles at all.
        HitResult::Passthrough
    }

    /// Update `HitRegionLocalState` for the given point.
    ///
    /// Called by the input pipeline (Stage 2) immediately after hit-testing to
    /// provide local visual feedback without waiting for the owning agent.
    /// Sets `hovered = true` on the newly-hit node and `hovered = false` on the
    /// previous hover node (if it changed).
    ///
    /// `prev_hover` — the node that was previously hovered (cleared on transition).
    /// `result`     — the current hit-test result.
    ///
    /// Returns the newly-hovered node ID (if any) for the caller to track.
    pub fn update_hover_state(
        &mut self,
        prev_hover: Option<SceneId>,
        result: &HitResult,
    ) -> Option<SceneId> {
        // Clear old hover.
        if let Some(old_id) = prev_hover {
            if let Some(state) = self.hit_region_states.get_mut(&old_id) {
                state.hovered = false;
            }
        }
        // Set new hover.  Use entry().or_insert_with() so that HitRegionNodes
        // inserted directly into `self.nodes` (e.g. in multi-node trees whose
        // children were not routed through `set_tile_root`) still get their
        // local state initialised on first hit rather than silently failing.
        let new_hover = if let HitResult::NodeHit { node_id, .. } = result {
            let state = self
                .hit_region_states
                .entry(*node_id)
                .or_insert_with(|| HitRegionLocalState::new(*node_id));
            state.hovered = true;
            Some(*node_id)
        } else {
            None
        };
        new_hover
    }

    /// Update pressed state for a node.
    ///
    /// Call with `pressed = true` on PointerDown and `pressed = false` on
    /// PointerUp / capture release.  No-op if the node has no local state entry.
    pub fn update_pressed_state(&mut self, node_id: SceneId, pressed: bool) {
        if let Some(state) = self.hit_region_states.get_mut(&node_id) {
            state.pressed = pressed;
        }
    }

    /// Update focused state for a node.
    ///
    /// The focus state machine is owned by the input epic; this helper allows
    /// the compositor to reflect focus changes into local state without a full
    /// state-machine transition.
    pub fn update_focused_state(&mut self, node_id: SceneId, focused: bool) {
        if let Some(state) = self.hit_region_states.get_mut(&node_id) {
            state.focused = focused;
        }
    }

    fn hit_test_node(&self, node_id: SceneId, x: f32, y: f32) -> Option<SceneId> {
        let node = self.nodes.get(&node_id)?;

        // Check children in reverse order (last child = front-most) — depth first.
        for child_id in node.children.iter().rev() {
            if let Some(hit) = self.hit_test_node(*child_id, x, y) {
                return Some(hit);
            }
        }

        // Check this node — only HitRegionNode with accepts_pointer qualifies.
        match &node.data {
            NodeData::HitRegion(hr) if hr.accepts_pointer && hr.bounds.contains_point(x, y) => {
                Some(node_id)
            }
            _ => None,
        }
    }

    // ─── Zone operations ─────────────────────────────────────────────────

    /// Register a zone definition in the zone registry.
    pub fn register_zone(&mut self, zone: ZoneDefinition) {
        self.zone_registry.register(zone);
        self.version += 1;
    }

    /// Unregister a zone by name. Returns the removed definition if found.
    pub fn unregister_zone(&mut self, name: &str) -> Option<ZoneDefinition> {
        let removed = self.zone_registry.unregister(name);
        if removed.is_some() {
            self.version += 1;
        }
        removed
    }

    /// Publish content to a zone. Applies contention policy.
    ///
    /// Token validation is out-of-scope for the pure scene graph layer;
    /// callers (e.g., the gRPC server) must validate the token before calling this.
    ///
    /// # Arguments
    /// - `zone_name` — zone type name, resolved in the global `zone_registry` (v1: publishes
    ///   are global, not tab-scoped; tab-scoped zone instances are a post-v1 feature)
    /// - `content` — content payload; must match one of the zone's accepted_media_types
    /// - `publisher_namespace` — the publishing agent's namespace
    /// - `merge_key` — key for MergeByKey contention (ignored for other policies)
    /// - `expires_at_wall_us` — optional wall-clock expiry (µs since epoch)
    /// - `content_classification` — optional opaque content classification tag
    pub fn publish_to_zone(
        &mut self,
        zone_name: &str,
        content: ZoneContent,
        publisher_namespace: &str,
        merge_key: Option<String>,
        expires_at_wall_us: Option<u64>,
        content_classification: Option<String>,
    ) -> Result<(), ValidationError> {
        // Check zone exists and content type is accepted
        let (contention_policy, max_publishers, accepted) = {
            let zone = self
                .zone_registry
                .get_by_name(zone_name)
                .ok_or_else(|| ValidationError::ZoneNotFound { name: zone_name.to_string() })?;
            let accepted = Self::content_media_type(&content)
                .map(|mt| zone.accepted_media_types.contains(&mt))
                .unwrap_or(true);
            (zone.contention_policy, zone.max_publishers, accepted)
        };

        if !accepted {
            return Err(ValidationError::ZoneMediaTypeMismatch {
                zone: zone_name.to_string(),
            });
        }

        let now_ms = self.clock.now_millis();
        let record = ZonePublishRecord {
            zone_name: zone_name.to_string(),
            publisher_namespace: publisher_namespace.to_string(),
            content,
            published_at_ms: now_ms,
            merge_key: merge_key.clone(),
            expires_at_wall_us,
            content_classification,
        };

        let publishes = self
            .zone_registry
            .active_publishes
            .entry(zone_name.to_string())
            .or_default();

        match contention_policy {
            ContentionPolicy::LatestWins => {
                // Replace all with the single new record
                *publishes = vec![record];
            }
            ContentionPolicy::Replace => {
                // Single occupant: evict current and replace
                *publishes = vec![record];
            }
            ContentionPolicy::Stack { max_depth } => {
                // Check publisher count limit
                let publisher_count = publishes
                    .iter()
                    .filter(|r| r.publisher_namespace == publisher_namespace)
                    .count() as u32;
                if publisher_count >= max_publishers {
                    return Err(ValidationError::ZoneMaxPublishersReached {
                        zone: zone_name.to_string(),
                        max: max_publishers,
                    });
                }
                publishes.push(record);
                // Trim oldest if stack exceeds max_depth
                let max = max_depth as usize;
                if publishes.len() > max {
                    let excess = publishes.len() - max;
                    publishes.drain(0..excess);
                }
            }
            ContentionPolicy::MergeByKey { max_keys } => {
                let key = merge_key.clone().unwrap_or_default();
                // Replace existing entry with same key
                if let Some(pos) = publishes.iter().position(|r| {
                    r.merge_key.as_deref().unwrap_or("") == key.as_str()
                }) {
                    publishes[pos] = record;
                } else {
                    // Check key count limit
                    if publishes.len() >= max_keys as usize {
                        return Err(ValidationError::ZoneMaxKeysReached {
                            zone: zone_name.to_string(),
                            max: max_keys as u32,
                        });
                    }
                    publishes.push(record);
                }
            }
        }

        self.version += 1;
        Ok(())
    }

    /// Publish content to a zone with lease-state enforcement.
    ///
    /// This is the lease-aware variant of `publish_to_zone`. It looks up the
    /// active lease for `publisher_namespace` and enforces spec
    /// §Requirement: Zone Publish Requires Active Lease (lines 213–242):
    ///
    /// - ACTIVE lease → accepted.
    /// - ORPHANED lease → rejected with `ZonePublishLeaseOrphaned`; existing
    ///   content remains visible with stale badge (spec lines 231–233).
    /// - SUSPENDED lease → rejected with `ZonePublishSafeModeActive`
    ///   (spec line 227).
    /// - Terminal or missing lease → rejected with `ZonePublishLeaseNotFound`
    ///   or `ZonePublishLeaseNotActive`.
    ///
    /// Callers that do not hold a lease (e.g., system/chrome publishers) should
    /// use the unchecked `publish_to_zone` directly.
    pub fn publish_to_zone_with_lease(
        &mut self,
        zone_name: &str,
        content: ZoneContent,
        publisher_namespace: &str,
        merge_key: Option<String>,
    ) -> Result<(), ValidationError> {
        use crate::lease::orphan::ZonePublishResult;

        // Prefer ACTIVE lease for this namespace. If no ACTIVE lease exists, find
        // the first non-terminal lease for error reporting. If only terminal leases
        // exist, return LEASE_NOT_ACTIVE (per spec line 214: inactive lease →
        // LEASE_NOT_ACTIVE). Return LEASE_NOT_FOUND only if no lease at all.
        let all_leases_for_ns: Vec<_> = self
            .leases
            .values()
            .filter(|l| l.namespace == publisher_namespace)
            .map(|l| l.state)
            .collect();

        let lease_state = if all_leases_for_ns.is_empty() {
            // No lease at all.
            None
        } else {
            // Prefer Active; fall back to the first non-terminal; otherwise use first terminal.
            all_leases_for_ns
                .iter()
                .copied()
                .find(|&s| s == LeaseState::Active)
                .or_else(|| all_leases_for_ns.iter().copied().find(|s| !s.is_terminal()))
                .or_else(|| all_leases_for_ns.first().copied())
        };

        match lease_state {
            None => {
                // No lease whatsoever (namespace has never held a lease).
                return Err(ValidationError::ZonePublishLeaseNotFound {
                    namespace: publisher_namespace.to_string(),
                });
            }
            Some(state) => {
                // Convert types::LeaseState to lease::LeaseState for the check.
                // The check function uses the lease mod's LeaseState, which
                // mirrors the types.rs version.
                let result = match state {
                    LeaseState::Active => ZonePublishResult::Accepted,
                    LeaseState::Orphaned | LeaseState::Disconnected => {
                        ZonePublishResult::RejectedLeaseOrphaned
                    }
                    LeaseState::Suspended => ZonePublishResult::RejectedSafeModeActive,
                    _ => ZonePublishResult::RejectedLeaseTerminal,
                };
                match result {
                    ZonePublishResult::Accepted => {} // fall through to publish
                    ZonePublishResult::RejectedLeaseOrphaned => {
                        return Err(ValidationError::ZonePublishLeaseOrphaned {
                            namespace: publisher_namespace.to_string(),
                        });
                    }
                    ZonePublishResult::RejectedSafeModeActive => {
                        return Err(ValidationError::ZonePublishSafeModeActive {
                            namespace: publisher_namespace.to_string(),
                        });
                    }
                    ZonePublishResult::RejectedLeaseTerminal => {
                        return Err(ValidationError::ZonePublishLeaseNotActive {
                            namespace: publisher_namespace.to_string(),
                            state: format!("{:?}", state),
                        });
                    }
                }
            }
        }

        // Lease is Active — delegate to unchecked publish.
        self.publish_to_zone(zone_name, content, publisher_namespace, merge_key, None, None)
    }

    /// Budget-driven revocation: transitions all non-terminal session leases to
    /// REVOKED, clears tiles, clears zone publications.
    ///
    /// Spec §Post-Revocation Resource Cleanup (lines 253–260):
    /// - Bypasses the grace period entirely.
    /// - Caller is responsible for sending `LeaseResponse{revoke_reason=BUDGET_POLICY}`
    ///   and then waiting `POST_REVOCATION_FREE_DELAY_MS` before calling
    ///   `finalize_budget_revocation`.
    ///
    /// Returns the cleanup spec (containing the free delay) for each revoked lease.
    pub fn initiate_budget_revocation(
        &mut self,
        session_namespace: &str,
    ) -> Vec<crate::lease::cleanup::PostRevocationCleanupSpec> {
        use crate::lease::cleanup::{PostRevocationCleanupSpec, RevocationKind};
        let now_ms = self.clock.now_millis();

        // Collect all non-terminal leases for this namespace.
        let to_revoke: Vec<SceneId> = self
            .leases
            .values()
            .filter(|l| l.namespace == session_namespace && !l.state.is_terminal())
            .map(|l| l.id)
            .collect();

        let mut specs = Vec::new();
        for lease_id in to_revoke {
            // Transition to REVOKED (bypasses grace — no orphan path).
            if let Some(lease) = self.leases.get_mut(&lease_id) {
                lease.state = LeaseState::Revoked;
            }
            // Tiles will be freed after the 100ms delay by finalize_budget_revocation.
            // Budget revocation bypasses the orphan/disconnection path, so tiles
            // do not receive a DisconnectionBadge — they are simply marked for
            // pending removal (visual_hint remains None; compositor will not render
            // them once removed by finalize_budget_revocation).

            // Clear zone publications immediately on REVOKED transition.
            // Spec §Requirement: Lease Revocation Clears Zone Publications
            // (lines 235–242): zone pubs must be cleared when lease is REVOKED/EXPIRED.
            // Tile/node resources are deferred by the 100ms delay; zone pubs are not.
            if let Some(lease) = self.leases.get(&lease_id) {
                let ns = lease.namespace.clone();
                self.clear_zone_publications_for_namespace(&ns);
            }
            specs.push(PostRevocationCleanupSpec::new(
                lease_id,
                session_namespace,
                RevocationKind::BudgetPolicy,
                now_ms,
            ));
        }

        if !specs.is_empty() {
            self.version += 1;
        }
        specs
    }

    /// Finalize budget revocation: remove tiles and zone publications for
    /// all leases in the cleanup specs that are ready to free.
    ///
    /// Must be called after `POST_REVOCATION_FREE_DELAY_MS` has elapsed
    /// (spec line 254: "free all resources after a 100ms delay").
    ///
    /// Returns the number of specs that were finalized.
    pub fn finalize_budget_revocation(
        &mut self,
        specs: &[crate::lease::cleanup::PostRevocationCleanupSpec],
        now_ms: u64,
    ) -> usize {
        let mut finalized = 0;
        for spec in specs {
            if spec.is_ready_to_free(now_ms) {
                // Remove tiles
                let tile_ids: Vec<SceneId> = self
                    .tiles
                    .values()
                    .filter(|t| t.lease_id == spec.lease_id)
                    .map(|t| t.id)
                    .collect();
                for tid in tile_ids {
                    self.remove_tile_and_nodes(tid);
                }
                // Clear zone publications
                self.clear_zone_publications_for_namespace(&spec.session_namespace);
                finalized += 1;
            }
        }
        if finalized > 0 {
            self.version += 1;
        }
        finalized
    }

    /// Clear all active publishes for a zone (regardless of publisher).
    ///
    /// This removes ALL publications from the zone. For per-publisher clearing,
    /// use [`clear_zone_for_publisher`].
    pub fn clear_zone(&mut self, zone_name: &str) -> Result<(), ValidationError> {
        if !self.zone_registry.zones.contains_key(zone_name) {
            return Err(ValidationError::ZoneNotFound { name: zone_name.to_string() });
        }
        self.zone_registry.active_publishes.remove(zone_name);
        self.version += 1;
        Ok(())
    }

    /// Clear all active publishes for a zone made by a specific publisher.
    ///
    /// Per spec: "ClearZone clears all publications by the agent in the specified zone."
    /// If no publications exist for the publisher, this is a no-op (but still succeeds).
    pub fn clear_zone_for_publisher(
        &mut self,
        zone_name: &str,
        publisher_namespace: &str,
    ) -> Result<(), ValidationError> {
        if !self.zone_registry.zones.contains_key(zone_name) {
            return Err(ValidationError::ZoneNotFound { name: zone_name.to_string() });
        }
        if let Some(publishes) = self.zone_registry.active_publishes.get_mut(zone_name) {
            let before = publishes.len();
            publishes.retain(|r| r.publisher_namespace != publisher_namespace);
            if publishes.len() != before {
                self.version += 1;
            }
        }
        Ok(())
    }

    /// Map ZoneContent to its ZoneMediaType, if deterministic.
    fn content_media_type(content: &ZoneContent) -> Option<ZoneMediaType> {
        match content {
            ZoneContent::StreamText(_) => Some(ZoneMediaType::StreamText),
            ZoneContent::Notification(_) => Some(ZoneMediaType::ShortTextWithIcon),
            ZoneContent::StatusBar(_) => Some(ZoneMediaType::KeyValuePairs),
            ZoneContent::SolidColor(_) => Some(ZoneMediaType::SolidColor),
            ZoneContent::StaticImage(_) => Some(ZoneMediaType::StaticImage),
            ZoneContent::VideoSurfaceRef(_) => Some(ZoneMediaType::VideoSurfaceRef),
        }
    }

    // ─── Queries ─────────────────────────────────────────────────────────

    /// Snapshot the entire scene graph as JSON.
    pub fn snapshot_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize a scene graph from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Take a deterministic full scene snapshot at the current sequence number.
    ///
    /// Implements RFC 0001 §4.1 — produces a complete, deterministic serialization
    /// of the scene graph. All maps use BTreeMap for deterministic iteration.
    ///
    /// # Checksum
    /// The returned `SceneGraphSnapshot.checksum` is a BLAKE3 hash (hex-encoded) computed
    /// over the canonical JSON of the snapshot with the checksum field set to `""`.
    /// Use [`SceneGraphSnapshot::verify_checksum`] to verify after deserialization.
    ///
    /// # Clock arguments
    /// `wall_us` is UTC wall-clock microseconds since epoch (u64). `mono_us` is
    /// monotonic microseconds since process start.
    ///
    /// # v1 Constraints
    /// - Resources are referenced by ResourceId only; no blob data is included.
    /// - effective_geometry is NOT included (post-v1 per spec line 360).
    /// - Incremental diff is NOT available (snapshot-only reconnect in v1).
    pub fn take_snapshot(&self, wall_us: u64, mono_us: u64) -> SceneGraphSnapshot {
        // Tabs: keyed by display_order for deterministic ordering.
        let tabs: std::collections::BTreeMap<u32, Tab> = self
            .tabs
            .values()
            .map(|t| (t.display_order, t.clone()))
            .collect();

        // Tiles: keyed by SceneId (BTreeMap — SceneId implements Ord).
        let tiles: std::collections::BTreeMap<SceneId, Tile> = self
            .tiles
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();

        // Nodes: keyed by SceneId.
        let nodes: std::collections::BTreeMap<SceneId, Node> = self
            .nodes
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();

        // Zone registry: BTreeMap for both zone_types and active_publications.
        let zone_types: std::collections::BTreeMap<String, ZoneDefinition> = self
            .zone_registry
            .zones
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Zone instances: intentionally empty in v1.
        // In v1 the zone_registry does not store ZoneInstance directly —
        // instance binding is implicit (one per tab per zone type when a zone is loaded).
        // Consumers of this snapshot MUST NOT rely on zone_instances being populated;
        // any instance bindings must be derived from zone_types and current tab/node state.
        // Post-v1: explicit ZoneInstance tracking will populate this field.
        let zone_instances: Vec<ZoneInstance> = Vec::new();

        // Active publications: BTreeMap keyed by zone name; each Vec is already
        // ordered by insertion (policy-enforced). Sort each Vec with a total order
        // to guarantee determinism: published_at_ms → publisher_namespace → merge_key.
        // The merge_key tie-breaker ensures records that share a timestamp and namespace
        // (e.g., MergeByKey records) are still ordered deterministically.
        let active_publications: std::collections::BTreeMap<String, Vec<ZonePublishRecord>> = self
            .zone_registry
            .active_publishes
            .iter()
            .map(|(zone_name, records)| {
                let mut sorted = records.clone();
                sorted.sort_by(|a, b| {
                    a.published_at_ms
                        .cmp(&b.published_at_ms)
                        .then_with(|| a.publisher_namespace.cmp(&b.publisher_namespace))
                        .then_with(|| a.merge_key.cmp(&b.merge_key))
                });
                (zone_name.clone(), sorted)
            })
            .collect();

        let zone_registry = SceneGraphZoneRegistry {
            zone_types,
            zone_instances,
            active_publications,
        };

        // Build the snapshot with a placeholder checksum first.
        let mut snapshot = SceneGraphSnapshot {
            sequence: self.sequence_number,
            snapshot_wall_us: wall_us,
            snapshot_mono_us: mono_us,
            tabs,
            tiles,
            nodes,
            zone_registry,
            active_tab: self.active_tab,
            checksum: String::new(),
        };

        // Compute and assign the checksum over the canonical content.
        snapshot.checksum = snapshot.compute_checksum();
        snapshot
    }

    /// Count total nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Count total tiles in the graph.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    // ─── Sequence number (RFC 0001 §3.5) ────────────────────────────────

    /// Advance the sequence counter and return the new value.
    ///
    /// Called by the mutation pipeline on every successful batch commit.
    /// Sequence numbers are strictly monotonically increasing u64 values.
    pub(crate) fn next_sequence_number(&mut self) -> u64 {
        self.sequence_number += 1;
        self.sequence_number
    }

    // ─── Clock accessor ──────────────────────────────────────────────────

    /// Return the current time in milliseconds from the injected clock.
    pub(crate) fn now_millis(&self) -> u64 {
        self.clock.now_millis()
    }
}


fn now_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Decision returned by `SceneGraph::evaluate_sync_group_commit`.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncGroupCommitDecision {
    /// Commit the listed tiles' pending mutations this frame.
    Commit { tiles: Vec<SceneId> },
    /// Defer the entire group to the next frame (AllOrDefer policy).
    Defer,
    /// Force-commit with the listed tiles after exhausting max_deferrals.
    /// The compositor should emit a `sync_group_force_commit` telemetry event.
    ForceCommit { tiles: Vec<SceneId> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::TestClock;

    /// Convenience: build a SceneGraph backed by a TestClock starting at t=1000ms.
    fn scene_with_test_clock() -> (SceneGraph, TestClock) {
        let clock = TestClock::new(1_000);
        let scene =
            SceneGraph::new_with_clock(1920.0, 1080.0, Arc::new(clock.clone()));
        (scene, clock)
    }

    #[test]
    fn test_create_scene_with_tab_and_tiles() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);

        // Create a tab
        let tab_id = scene.create_tab("Main", 0).unwrap();
        assert_eq!(scene.active_tab, Some(tab_id));

        // Grant a lease
        let lease_id = scene.grant_lease(
            "test-agent",
            60_000,
            vec![Capability::CreateTile, Capability::CreateNode],
        );

        // Create two tiles
        let tile1_id = scene
            .create_tile(tab_id, "test-agent", lease_id, Rect::new(10.0, 10.0, 400.0, 300.0), 1)
            .unwrap();

        let tile2_id = scene
            .create_tile(tab_id, "test-agent", lease_id, Rect::new(420.0, 10.0, 400.0, 300.0), 2)
            .unwrap();

        assert_eq!(scene.tile_count(), 2);

        // Add nodes
        let text_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "Hello, tze_hud!".to_string(),
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
                font_size_px: 24.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: Some(Rgba::new(0.1, 0.1, 0.2, 1.0)),
                alignment: TextAlign::Center,
                overflow: TextOverflow::Clip,
            }),
        };
        scene.set_tile_root(tile1_id, text_node).unwrap();

        let hit_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                interaction_id: "btn-click".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        scene.set_tile_root(tile2_id, hit_node.clone()).unwrap();

        assert_eq!(scene.node_count(), 2);
        assert!(scene.hit_region_states.contains_key(&hit_node.id));
    }

    #[test]
    fn test_hit_test() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        let tile_id = scene
            .create_tile(tab_id, "test", lease_id, Rect::new(100.0, 100.0, 400.0, 300.0), 1)
            .unwrap();

        let hr_node_id = SceneId::new();
        let hit_node = Node {
            id: hr_node_id,
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(50.0, 50.0, 200.0, 100.0),
                interaction_id: "btn".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };
        scene.set_tile_root(tile_id, hit_node).unwrap();

        // Hit the hit region (tile at 100,100; region at 50,50 within tile = 150,150 global)
        let result = scene.hit_test(200.0, 180.0);
        assert_eq!(
            result,
            HitResult::NodeHit {
                tile_id,
                node_id: hr_node_id,
                interaction_id: "btn".to_string(),
            }
        );

        // Miss the hit region but hit the tile
        let result = scene.hit_test(110.0, 110.0);
        assert_eq!(result, HitResult::TileHit { tile_id });

        // Miss everything
        let result = scene.hit_test(10.0, 10.0);
        assert_eq!(result, HitResult::Passthrough);
    }

    #[test]
    fn test_snapshot_roundtrip() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        let json = scene.snapshot_json().unwrap();
        let restored = SceneGraph::from_json(&json).unwrap();

        assert_eq!(scene.tile_count(), restored.tile_count());
        assert_eq!(scene.active_tab, restored.active_tab);
        assert_eq!(scene.version, restored.version);
    }

    #[test]
    fn test_lease_expiry() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // Grant a lease with a 500 ms TTL.
        // Clock is at t=1000; lease expires at t=1500.
        let lease_id = scene.grant_lease("test", 500, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        assert_eq!(scene.tile_count(), 1);

        // Before expiry: clock still at t=1000, lease lives.
        let expired = scene.expire_leases();
        assert_eq!(expired.len(), 0);
        assert_eq!(scene.tile_count(), 1);

        // Advance past the TTL.
        clock.advance(501);
        let expired = scene.expire_leases();
        assert_eq!(expired.len(), 1);
        assert_eq!(scene.tile_count(), 0);
    }

    #[test]
    fn test_tab_created_at_uses_clock() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        assert_eq!(scene.tabs[&tab_id].created_at_ms, 1_000);

        // Advancing the clock does NOT retroactively change existing timestamps.
        clock.advance(100);
        assert_eq!(scene.tabs[&tab_id].created_at_ms, 1_000);
    }

    #[test]
    fn test_renew_lease_uses_clock() {
        let (mut scene, clock) = scene_with_test_clock();
        // Clock at t=1000.
        let lease_id = scene.grant_lease("test", 5_000, vec![]);
        assert_eq!(scene.leases[&lease_id].granted_at_ms, 1_000);

        // Advance clock then renew.
        clock.advance(2_000);
        scene.renew_lease(lease_id, 10_000).unwrap();
        assert_eq!(scene.leases[&lease_id].granted_at_ms, 3_000);
        assert_eq!(scene.leases[&lease_id].ttl_ms, 10_000);
    }

    #[test]
    fn test_lease_revocation_cleans_tiles() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(200.0, 0.0, 100.0, 100.0), 2)
            .unwrap();

        assert_eq!(scene.tile_count(), 2);
        scene.revoke_lease(lease_id).unwrap();
        assert_eq!(scene.tile_count(), 0);
        // Revoked leases remain in the map with terminal state
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Revoked);
    }

    #[test]
    fn test_visible_tiles_sorted_by_z_order() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        scene.create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 5).unwrap();
        scene.create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1).unwrap();
        scene.create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 3).unwrap();

        let visible = scene.visible_tiles();
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].z_order, 1);
        assert_eq!(visible[1].z_order, 3);
        assert_eq!(visible[2].z_order, 5);
    }

    // ─── Zone tests ───────────────────────────────────────────────────────

    fn make_subtitle_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_string(),
            description: "Subtitle overlay".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 48.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 2,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        }
    }

    fn make_notification_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "notifications".to_string(),
            description: "Notification stack".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 3 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: Some(5_000),
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        }
    }

    fn make_status_bar_zone() -> ZoneDefinition {
        ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_string(),
            description: "Status bar".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 8 },
            max_publishers: 8,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Chrome,
        }
    }

    fn dummy_token() -> ZonePublishToken {
        ZonePublishToken { token: vec![0xDE, 0xAD, 0xBE, 0xEF] }
    }

    #[test]
    fn test_zone_register_unregister() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone = make_subtitle_zone();

        scene.register_zone(zone.clone());
        assert!(scene.zone_registry.get_by_name("subtitle").is_some());

        let removed = scene.unregister_zone("subtitle");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "subtitle");
        assert!(scene.zone_registry.get_by_name("subtitle").is_none());
    }

    #[test]
    fn test_zone_query_by_name() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.register_zone(make_notification_zone());

        let zone = scene.zone_registry.get_by_name("subtitle").unwrap();
        assert_eq!(zone.name, "subtitle");
        assert!(scene.zone_registry.get_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_zone_query_by_media_type() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.register_zone(make_notification_zone());

        let stream_text_zones = scene.zone_registry.zones_accepting(ZoneMediaType::StreamText);
        assert_eq!(stream_text_zones.len(), 1);
        assert_eq!(stream_text_zones[0].name, "subtitle");

        let notif_zones = scene.zone_registry.zones_accepting(ZoneMediaType::ShortTextWithIcon);
        assert_eq!(notif_zones.len(), 1);
        assert_eq!(notif_zones[0].name, "notifications");
    }

    #[test]
    fn test_default_zones_populated() {
        let registry = ZoneRegistry::with_defaults();
        assert!(registry.get_by_name("status-bar").is_some());
        assert!(registry.get_by_name("notification-area").is_some());
        assert!(registry.get_by_name("subtitle").is_some());
    }

    #[test]
    fn test_zone_publish_not_found() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = scene.publish_to_zone(
            "nonexistent",
            ZoneContent::StreamText("hello".to_string()),
            "agent",
            None,
            None,
            None,
        );
        assert!(matches!(result, Err(ValidationError::ZoneNotFound { .. })));
    }

    #[test]
    fn test_zone_publish_media_type_mismatch() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone()); // accepts StreamText only

        let result = scene.publish_to_zone(
            "subtitle",
            ZoneContent::Notification(NotificationPayload {
                text: "Hello".to_string(),
                icon: "".to_string(),
                urgency: 1,
            }),
            "agent",
            None,
            None,
            None,
        );
        assert!(matches!(result, Err(ValidationError::ZoneMediaTypeMismatch { .. })));
    }

    #[test]
    fn test_contention_latest_wins() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        scene.publish_to_zone("subtitle", ZoneContent::StreamText("first".to_string()), "a1", None, None, None).unwrap();
        scene.publish_to_zone("subtitle", ZoneContent::StreamText("second".to_string()), "a2", None, None, None).unwrap();

        let publishes = scene.zone_registry.active_for_zone("subtitle");
        assert_eq!(publishes.len(), 1);
        assert_eq!(publishes[0].content, ZoneContent::StreamText("second".to_string()));
        assert_eq!(publishes[0].publisher_namespace, "a2");
    }

    #[test]
    fn test_contention_stack() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_notification_zone()); // Stack { max_depth: 3 }

        let notification = |text: &str| ZoneContent::Notification(NotificationPayload {
            text: text.to_string(),
            icon: "".to_string(),
            urgency: 1,
        });

        scene.publish_to_zone("notifications", notification("msg1"), "a1", None, None, None).unwrap();
        scene.publish_to_zone("notifications", notification("msg2"), "a2", None, None, None).unwrap();
        scene.publish_to_zone("notifications", notification("msg3"), "a3", None, None, None).unwrap();

        let publishes = scene.zone_registry.active_for_zone("notifications");
        assert_eq!(publishes.len(), 3);

        // 4th publish should trim the oldest
        scene.publish_to_zone("notifications", notification("msg4"), "a4", None, None, None).unwrap();
        let publishes = scene.zone_registry.active_for_zone("notifications");
        assert_eq!(publishes.len(), 3);
        // Oldest (msg1) should be gone, newest (msg4) at end
        if let ZoneContent::Notification(n) = &publishes[0].content {
            assert_eq!(n.text, "msg2");
        } else {
            panic!("expected Notification");
        }
        if let ZoneContent::Notification(n) = &publishes[2].content {
            assert_eq!(n.text, "msg4");
        } else {
            panic!("expected Notification");
        }
    }

    // ─── Sync Group Tests ────────────────────────────────────────────────

    fn make_scene_with_tiles(count: usize) -> (SceneGraph, SceneId, Vec<SceneId>) {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateTile]);
        let mut tile_ids = Vec::new();
        for i in 0..count {
            let tile_id = scene
                .create_tile(
                    tab_id,
                    "agent",
                    lease_id,
                    Rect::new(i as f32 * 110.0, 0.0, 100.0, 100.0),
                    i as u32,
                )
                .unwrap();
            tile_ids.push(tile_id);
        }
        (scene, tab_id, tile_ids)
    }

    #[test]
    fn test_create_sync_group() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);

        let group_id = scene
            .create_sync_group(
                Some("test-group".to_string()),
                "agent",
                SyncCommitPolicy::AllOrDefer,
                3,
            )
            .unwrap();

        assert_eq!(scene.sync_group_count(), 1);
        let group = scene.sync_groups.get(&group_id).unwrap();
        assert_eq!(group.name, Some("test-group".to_string()));
        assert_eq!(group.owner_namespace, "agent");
        assert_eq!(group.commit_policy, SyncCommitPolicy::AllOrDefer);
        assert_eq!(group.max_deferrals, 3);
        assert!(group.members.is_empty());
    }

    #[test]
    fn test_delete_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);

        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();

        // Join both tiles
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Deleting the group should release tiles
        scene.delete_sync_group(group_id).unwrap();
        assert_eq!(scene.sync_group_count(), 0);

        // Tiles should have no sync_group reference
        assert_eq!(scene.tiles[&tiles[0]].sync_group, None);
        assert_eq!(scene.tiles[&tiles[1]].sync_group, None);
    }

    #[test]
    fn test_delete_nonexistent_sync_group_errors() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);
        let fake_id = SceneId::new();
        let result = scene.delete_sync_group(fake_id);
        assert!(matches!(result, Err(ValidationError::SyncGroupNotFound { .. })));
    }

    #[test]
    fn test_join_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();

        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        assert_eq!(scene.sync_groups[&group_id].members.len(), 2);
        assert_eq!(scene.tiles[&tiles[0]].sync_group, Some(group_id));
        assert_eq!(scene.tiles[&tiles[1]].sync_group, Some(group_id));
    }

    #[test]
    fn test_join_replaces_old_group_membership() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        let group_a = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();
        let group_b = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();

        scene.join_sync_group(tiles[0], group_a).unwrap();
        // Now join a different group — should leave group_a automatically
        scene.join_sync_group(tiles[0], group_b).unwrap();

        assert!(!scene.sync_groups[&group_a].members.contains(&tiles[0]));
        assert!(scene.sync_groups[&group_b].members.contains(&tiles[0]));
        assert_eq!(scene.tiles[&tiles[0]].sync_group, Some(group_b));
    }

    #[test]
    fn test_leave_sync_group() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();

        scene.join_sync_group(tiles[0], group_id).unwrap();
        assert!(scene.sync_groups[&group_id].members.contains(&tiles[0]));

        scene.leave_sync_group(tiles[0]).unwrap();
        assert!(!scene.sync_groups[&group_id].members.contains(&tiles[0]));
        assert_eq!(scene.tiles[&tiles[0]].sync_group, None);
        // Group still exists after tile leaves
        assert_eq!(scene.sync_group_count(), 1);
    }

    #[test]
    fn test_leave_when_not_in_group_is_noop() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(1);
        // No group created — tile has no sync_group; leave should succeed silently
        let result = scene.leave_sync_group(tiles[0]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_available_members_commit_policy() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AvailableMembers, 0)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has a pending mutation
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();

        // AvailableMembers: commit whatever is ready, no deferral
        match decision {
            SyncGroupCommitDecision::Commit { tiles: committed } => {
                assert_eq!(committed, vec![tiles[0]]);
            }
            other => panic!("Expected Commit, got {:?}", other),
        }
    }

    #[test]
    fn test_contention_merge_by_key() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_status_bar_zone()); // MergeByKey { max_keys: 8 }

        let kv = |k: &str, v: &str| {
            let mut entries = std::collections::HashMap::new();
            entries.insert(k.to_string(), v.to_string());
            ZoneContent::StatusBar(StatusBarPayload { entries })
        };

        // Publish with different keys
        scene.publish_to_zone("status-bar", kv("clock", "12:00"), "a1", Some("clock".to_string()), None, None).unwrap();
        scene.publish_to_zone("status-bar", kv("battery", "80%"), "a2", Some("battery".to_string()), None, None).unwrap();

        let publishes = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(publishes.len(), 2);

        // Update existing key "clock"
        scene.publish_to_zone("status-bar", kv("clock", "12:01"), "a1", Some("clock".to_string()), None, None).unwrap();
        let publishes = scene.zone_registry.active_for_zone("status-bar");
        assert_eq!(publishes.len(), 2); // Still 2 (clock replaced, battery retained)
        let clock = publishes.iter().find(|r| r.merge_key.as_deref() == Some("clock")).unwrap();
        if let ZoneContent::StatusBar(sb) = &clock.content {
            assert_eq!(sb.entries["clock"], "12:01");
        } else {
            panic!("expected StatusBar");
        }
    }

    #[test]
    fn test_contention_replace() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let zone = ZoneDefinition {
            id: SceneId::new(),
            name: "pip".to_string(),
            description: "Picture in picture".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.80,
                y_pct: 0.80,
                width_pct: 0.18,
                height_pct: 0.18,
            },
            accepted_media_types: vec![ZoneMediaType::SolidColor],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Replace,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        };
        scene.register_zone(zone);

        scene.publish_to_zone("pip", ZoneContent::SolidColor(Rgba::WHITE), "a1", None, None, None).unwrap();
        scene.publish_to_zone("pip", ZoneContent::SolidColor(Rgba::BLACK), "a2", None, None, None).unwrap();

        let publishes = scene.zone_registry.active_for_zone("pip");
        assert_eq!(publishes.len(), 1);
        assert_eq!(publishes[0].publisher_namespace, "a2");
    }

    #[test]
    fn test_clear_zone() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        scene.publish_to_zone("subtitle", ZoneContent::StreamText("hello".to_string()), "a1", None, None, None).unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 1);

        scene.clear_zone("subtitle").unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 0);
    }

    #[test]
    fn test_clear_zone_not_found() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let result = scene.clear_zone("nonexistent");
        assert!(matches!(result, Err(ValidationError::ZoneNotFound { .. })));
    }

    #[test]
    fn test_zone_registry_snapshot() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.publish_to_zone("subtitle", ZoneContent::StreamText("hi".to_string()), "a1", None, None, None).unwrap();

        let snap = scene.zone_registry.snapshot();
        assert_eq!(snap.zones.len(), 1);
        assert_eq!(snap.active_publishes.len(), 1);
        assert_eq!(snap.active_publishes[0].zone_name, "subtitle");
    }

    #[test]
    fn test_zone_publish_via_mutation_batch() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());

        use crate::mutation::{MutationBatch, SceneMutation};

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![
                SceneMutation::PublishToZone {
                    zone_name: "subtitle".to_string(),
                    content: ZoneContent::StreamText("batch publish".to_string()),
                    publish_token: dummy_token(),
                    merge_key: None,
                    expires_at_wall_us: None,
                    content_classification: None,
                },
            ],
            timing_hints: None,
            lease_id: None,
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied, "batch should be applied");
        let publishes = scene.zone_registry.active_for_zone("subtitle");
        assert_eq!(publishes.len(), 1);
        assert_eq!(publishes[0].content, ZoneContent::StreamText("batch publish".to_string()));
    }

    #[test]
    fn test_clear_zone_via_mutation_batch() {
        // Per spec: ClearZone clears publications by THIS agent (batch.agent_namespace).
        // Publish as "agent", then clear as "agent" — should clear.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        scene.register_zone(make_subtitle_zone());
        scene.publish_to_zone("subtitle", ZoneContent::StreamText("hello".to_string()), "agent", None, None, None).unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 1);

        use crate::mutation::{MutationBatch, SceneMutation};

        let batch = MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "agent".to_string(),
            mutations: vec![
                SceneMutation::ClearZone {
                    zone_name: "subtitle".to_string(),
                    publish_token: dummy_token(),
                },
            ],
            timing_hints: None,
            lease_id: None,
        };

        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        // "agent" published, "agent" cleared — should be 0
        assert_eq!(scene.zone_registry.active_for_zone("subtitle").len(), 0);
    }

    #[test]
    fn test_clear_zone_per_publisher_only_affects_own_publishes() {
        // Publish as two agents; ClearZone from agent "a1" should only remove "a1"'s publish.
        // subtitle zone has max_publishers=2 for this test; use a zone that supports 2 publishers.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        // Use a Stack zone so both publishes can coexist
        let stack_zone = ZoneDefinition {
            id: SceneId::new(),
            name: "shared".to_string(),
            description: "Stack zone for publisher isolation test".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.0,
                y_pct: 0.0,
                width_pct: 1.0,
                height_pct: 0.1,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 4 },
            max_publishers: 4,
            transport_constraint: None,
            auto_clear_ms: None,
            ephemeral: false,
            layer_attachment: LayerAttachment::Content,
        };
        scene.register_zone(stack_zone);

        scene.publish_to_zone("shared", ZoneContent::StreamText("from a1".to_string()), "a1", None, None, None).unwrap();
        scene.publish_to_zone("shared", ZoneContent::StreamText("from a2".to_string()), "a2", None, None, None).unwrap();
        assert_eq!(scene.zone_registry.active_for_zone("shared").len(), 2);

        // Clear only "a1"'s publication
        scene.clear_zone_for_publisher("shared", "a1").unwrap();
        let pubs = scene.zone_registry.active_for_zone("shared");
        assert_eq!(pubs.len(), 1);
        assert_eq!(pubs[0].publisher_namespace, "a2");
    }

    #[test]
    fn test_all_or_defer_commits_when_all_ready() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);
        pending.insert(tiles[1]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();

        // All members ready → Commit
        match decision {
            SyncGroupCommitDecision::Commit { tiles: committed } => {
                assert_eq!(committed.len(), 2);
            }
            other => panic!("Expected Commit, got {:?}", other),
        }
        // Deferral counter should be reset to 0
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);
    }

    #[test]
    fn test_all_or_defer_defers_when_incomplete() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has a pending mutation
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        let decision = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 1);

        // Second deferral
        let decision2 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision2, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 2);

        // Third deferral
        let decision3 = scene
            .evaluate_sync_group_commit(group_id, &pending)
            .unwrap();
        assert_eq!(decision3, SyncGroupCommitDecision::Defer);
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 3);
    }

    #[test]
    fn test_all_or_defer_force_commits_after_max_deferrals() {
        let (mut scene, _tab, tiles) = make_scene_with_tiles(2);
        // max_deferrals = 2
        let group_id = scene
            .create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 2)
            .unwrap();
        scene.join_sync_group(tiles[0], group_id).unwrap();
        scene.join_sync_group(tiles[1], group_id).unwrap();

        // Only tile[0] has pending mutations — tile[1] is always missing
        let mut pending = std::collections::BTreeSet::new();
        pending.insert(tiles[0]);

        // Frame 1: deferral_count goes 0 → 1
        let d1 = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
        assert_eq!(d1, SyncGroupCommitDecision::Defer);

        // Frame 2: deferral_count goes 1 → 2
        let d2 = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
        assert_eq!(d2, SyncGroupCommitDecision::Defer);

        // Frame 3: deferral_count == max_deferrals (2) → force commit
        let d3 = scene.evaluate_sync_group_commit(group_id, &pending).unwrap();
        match d3 {
            SyncGroupCommitDecision::ForceCommit { tiles: committed } => {
                // Only tile[0] should be committed (tile[1] has no pending)
                assert_eq!(committed, vec![tiles[0]]);
            }
            other => panic!("Expected ForceCommit, got {:?}", other),
        }
        // Deferral counter reset after force-commit
        assert_eq!(scene.sync_groups[&group_id].deferral_count, 0);
    }

    #[test]
    fn test_sync_group_namespace_limit() {
        let (mut scene, _tab, _tiles) = make_scene_with_tiles(0);

        // Create 16 sync groups (the namespace limit)
        for i in 0..SceneGraph::MAX_SYNC_GROUPS_PER_NAMESPACE {
            scene
                .create_sync_group(
                    Some(format!("group-{}", i)),
                    "agent",
                    SyncCommitPolicy::AllOrDefer,
                    3,
                )
                .unwrap();
        }
        assert_eq!(scene.sync_group_count(), SceneGraph::MAX_SYNC_GROUPS_PER_NAMESPACE);

        // 17th should fail
        let result =
            scene.create_sync_group(None, "agent", SyncCommitPolicy::AllOrDefer, 3);
        assert!(matches!(result, Err(ValidationError::SyncGroupLimitExceeded { .. })));

        // A different namespace can still create groups
        let other_group = scene
            .create_sync_group(None, "other-agent", SyncCommitPolicy::AllOrDefer, 3);
        assert!(other_group.is_ok());
    }

    // ─── StaticImageNode tests ────────────────────────────────────────────

    /// Build a test `ResourceId` and decoded size for a w×h RGBA8 image.
    ///
    /// Per RS-4 ephemerality contract, `StaticImageNode` carries only the
    /// content-addressed `ResourceId` and the decoded byte count for budget
    /// accounting — no raw pixel data is embedded in the scene graph.
    fn make_test_image_resource(w: u32, h: u32) -> (ResourceId, u64) {
        // Compute a deterministic ResourceId from the dimensions (as a stand-in
        // for "the BLAKE3 hash of the actual pixel bytes").  In production this
        // would be the ResourceId returned by the resource store after upload.
        let fake_bytes: Vec<u8> = (0..w * h).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let resource_id = ResourceId::of(&fake_bytes);
        let decoded_bytes = u64::from(w * h * 4);
        (resource_id, decoded_bytes)
    }

    #[test]
    fn test_static_image_node_creation() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![Capability::CreateNode]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 400.0, 300.0), 1)
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(64, 48);
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 64,
                height: 48,
                decoded_bytes,
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 400.0, 300.0),
            }),
        };

        scene.set_tile_root(tile_id, node.clone()).unwrap();
        assert_eq!(scene.node_count(), 1);

        let stored = scene.nodes.get(&node.id).unwrap();
        if let NodeData::StaticImage(si) = &stored.data {
            assert_eq!(si.resource_id, resource_id);
            assert_eq!(si.width, 64);
            assert_eq!(si.height, 48);
            assert_eq!(si.decoded_bytes, 64u64 * 48 * 4);
            assert_eq!(si.fit_mode, ImageFitMode::Contain);
        } else {
            panic!("expected StaticImage node data");
        }
    }

    #[test]
    fn test_static_image_node_all_fit_modes() {
        // Verify all ImageFitMode variants are constructable and round-trip through JSON.
        let (resource_id, decoded_bytes) = make_test_image_resource(4, 4);
        for fit_mode in [
            ImageFitMode::Contain,
            ImageFitMode::Cover,
            ImageFitMode::Fill,
            ImageFitMode::ScaleDown,
        ] {
            let node_data = NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 4,
                height: 4,
                decoded_bytes,
                fit_mode,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            });
            let json = serde_json::to_string(&node_data).unwrap();
            // Acceptance (RS-4): snapshot must NOT contain raw blob data.
            assert!(
                !json.contains("image_data"),
                "snapshot JSON must not contain image_data blob"
            );
            let restored: NodeData = serde_json::from_str(&json).unwrap();
            if let NodeData::StaticImage(si) = restored {
                assert_eq!(si.fit_mode, fit_mode);
                assert_eq!(si.resource_id, resource_id);
            } else {
                panic!("wrong variant after JSON roundtrip");
            }
        }
    }

    #[test]
    fn test_static_image_node_snapshot_roundtrip() {
        let mut scene = SceneGraph::new(1280.0, 720.0);
        let tab_id = scene.create_tab("Tab", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(10.0, 10.0, 200.0, 150.0), 1)
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(16, 16);
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 16,
                height: 16,
                decoded_bytes,
                fit_mode: ImageFitMode::Cover,
                bounds: Rect::new(0.0, 0.0, 200.0, 150.0),
            }),
        };
        scene.set_tile_root(tile_id, node).unwrap();

        let json = scene.snapshot_json().unwrap();

        // Acceptance (RS-4): scene snapshot includes ResourceId references but NOT blob data.
        // The JSON must not contain raw pixel data.
        assert!(
            !json.contains("image_data"),
            "snapshot JSON must not embed raw image blob data (RS-4 ephemerality contract)"
        );

        let restored = SceneGraph::from_json(&json).unwrap();

        assert_eq!(scene.node_count(), restored.node_count());
        // Verify the node data survived the roundtrip.
        for (_id, n) in &restored.nodes {
            if let NodeData::StaticImage(si) = &n.data {
                assert_eq!(si.resource_id, resource_id,
                    "resource_id must survive snapshot roundtrip");
                assert_eq!(si.fit_mode, ImageFitMode::Cover);
                assert_eq!(si.width, 16);
                assert_eq!(si.height, 16);
                assert_eq!(si.decoded_bytes, decoded_bytes);
            }
        }
    }

    #[test]
    fn test_static_image_node_replace_with_set_tile_root() {
        // Verify that replacing a StaticImageNode via set_tile_root removes the old node.
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 60_000, vec![]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        let (resource_id, decoded_bytes) = make_test_image_resource(8, 8);
        let node1 = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id,
                width: 8,
                height: 8,
                decoded_bytes,
                fit_mode: ImageFitMode::Fill,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            }),
        };
        let node1_id = node1.id;
        scene.set_tile_root(tile_id, node1).unwrap();
        assert_eq!(scene.node_count(), 1);
        assert!(scene.nodes.contains_key(&node1_id));

        // Replace with a SolidColor node.
        let node2 = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            }),
        };
        scene.set_tile_root(tile_id, node2).unwrap();
        // Old image node should be gone.
        assert!(!scene.nodes.contains_key(&node1_id));
        assert_eq!(scene.node_count(), 1);
    }

    // ─── Lease State Machine Tests (RFC 0008) ───────────────────────────

    #[test]
    fn test_lease_state_defaults_to_active() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Active);
        assert!(scene.leases[&lease_id].is_active());
        assert!(scene.leases[&lease_id].is_mutations_allowed());
    }

    #[test]
    fn test_lease_suspend_from_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        clock.advance(10_000); // 10s elapsed
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Suspended);
        assert!(!lease.is_mutations_allowed());
        assert!(lease.suspended_at_ms.is_some());
        assert!(lease.ttl_remaining_at_suspend_ms.is_some());
        // 60_000 - 10_000 = 50_000 remaining at suspend
        assert_eq!(lease.ttl_remaining_at_suspend_ms, Some(50_000));
    }

    #[test]
    fn test_lease_suspend_invalid_from_non_active() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        // Suspend once (valid)
        scene.suspend_lease(&lease_id, 1000).unwrap();

        // Suspend again from Suspended state (invalid)
        let err = scene.suspend_lease(&lease_id, 2000).unwrap_err();
        assert!(matches!(err, LeaseError::InvalidTransition {
            from: LeaseState::Suspended,
            to: LeaseState::Suspended,
        }));
    }

    #[test]
    fn test_lease_resume_from_suspended() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        clock.advance(10_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        clock.advance(5_000); // 5s in suspended state
        scene.resume_lease(&lease_id, clock.now_millis()).unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Active);
        assert!(lease.is_mutations_allowed());
        assert!(lease.suspended_at_ms.is_none());
        assert!(lease.ttl_remaining_at_suspend_ms.is_none());
        // After resume: TTL should reflect the remaining time from suspension
        // remaining was 50_000 at suspend; now granted_at_ms is set to resume time
        // so remaining_ms(now) should be ~50_000
        assert_eq!(lease.remaining_ms(clock.now_millis()), 50_000);
    }

    #[test]
    fn test_lease_resume_invalid_from_active() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        let err = scene.resume_lease(&lease_id, 1000).unwrap_err();
        assert!(matches!(err, LeaseError::InvalidTransition {
            from: LeaseState::Active,
            to: LeaseState::Active,
        }));
    }

    #[test]
    fn test_lease_disconnect_from_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        clock.advance(5_000);
        scene.disconnect_lease(&lease_id, clock.now_millis()).unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Orphaned);
        assert!(!lease.is_mutations_allowed());
        assert_eq!(lease.disconnected_at_ms, Some(6_000)); // 1000 start + 5000
    }

    #[test]
    fn test_lease_disconnect_invalid_from_suspended() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        scene.suspend_lease(&lease_id, 1000).unwrap();

        let err = scene.disconnect_lease(&lease_id, 2000).unwrap_err();
        assert!(matches!(err, LeaseError::InvalidTransition {
            from: LeaseState::Suspended,
            to: LeaseState::Orphaned,
        }));
    }

    #[test]
    fn test_lease_reconnect_within_grace() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        clock.advance(5_000);
        scene.disconnect_lease(&lease_id, clock.now_millis()).unwrap();

        // Reconnect within the 30s grace period
        clock.advance(10_000);
        scene.reconnect_lease(&lease_id, clock.now_millis()).unwrap();

        let lease = &scene.leases[&lease_id];
        assert_eq!(lease.state, LeaseState::Active);
        assert!(lease.is_mutations_allowed());
        assert!(lease.disconnected_at_ms.is_none());
    }

    #[test]
    fn test_lease_reconnect_after_grace_fails() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 120_000, vec![]);

        clock.advance(5_000);
        scene.disconnect_lease(&lease_id, clock.now_millis()).unwrap();

        // Advance past the 30s grace period
        clock.advance(31_000);
        let err = scene.reconnect_lease(&lease_id, clock.now_millis()).unwrap_err();
        assert!(matches!(err, LeaseError::InvalidTransition { .. }));
    }

    #[test]
    fn test_lease_revoke_from_any_non_terminal() {
        let (mut scene, _clock) = scene_with_test_clock();

        // Revoke from Active
        let l1 = scene.grant_lease("t1", 60_000, vec![]);
        scene.leases.get_mut(&l1).unwrap().revoke().unwrap();
        assert_eq!(scene.leases[&l1].state, LeaseState::Revoked);

        // Revoke from Suspended
        let l2 = scene.grant_lease("t2", 60_000, vec![]);
        scene.leases.get_mut(&l2).unwrap().suspend(1000).unwrap();
        scene.leases.get_mut(&l2).unwrap().revoke().unwrap();
        assert_eq!(scene.leases[&l2].state, LeaseState::Revoked);

        // Revoke from Disconnected
        let l3 = scene.grant_lease("t3", 60_000, vec![]);
        scene.leases.get_mut(&l3).unwrap().disconnect(1000).unwrap();
        scene.leases.get_mut(&l3).unwrap().revoke().unwrap();
        assert_eq!(scene.leases[&l3].state, LeaseState::Revoked);
    }

    #[test]
    fn test_lease_revoke_from_terminal_fails() {
        let (mut scene, _clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        scene.leases.get_mut(&lease_id).unwrap().revoke().unwrap();

        // Already revoked — should fail
        let err = scene.leases.get_mut(&lease_id).unwrap().revoke().unwrap_err();
        assert!(matches!(err, LeaseError::InvalidTransition {
            from: LeaseState::Revoked,
            to: LeaseState::Revoked,
        }));
    }

    #[test]
    fn test_lease_is_expired_not_when_suspended() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 1_000, vec![]);

        // Suspend at t=500ms (halfway)
        clock.advance(500);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Advance well past TTL
        clock.advance(10_000);
        assert!(!scene.leases[&lease_id].is_expired(clock.now_millis()));
    }

    // ─── Budget Enforcement Tests ───────────────────────────────────────

    #[test]
    fn test_budget_tile_count_within_limit() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        // Default budget: max_tiles = 8. Create 1 tile — should be fine.
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert!(!result.budget_warning);
    }

    #[test]
    fn test_budget_tile_count_exceeds_limit() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        // Set budget to max 2 tiles
        scene.leases.get_mut(&lease_id).unwrap().resource_budget.max_tiles = 2;

        // Create 2 tiles (OK)
        for i in 0..2 {
            let batch = crate::mutation::MutationBatch {
                batch_id: SceneId::new(),
                agent_namespace: "test".to_string(),
                mutations: vec![crate::mutation::SceneMutation::CreateTile {
                    tab_id,
                    namespace: "test".to_string(),
                    lease_id,
                    bounds: Rect::new(i as f32 * 120.0, 0.0, 100.0, 100.0),
                    z_order: i + 1,
                }],
                timing_hints: None,
                lease_id: None,
            };
            let result = scene.apply_batch(&batch);
            assert!(result.applied);
        }

        // Create a 3rd tile — should be rejected
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(240.0, 0.0, 100.0, 100.0),
                z_order: 3,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        assert!(result.error.is_some());
        assert_eq!(scene.tile_count(), 2);
    }

    #[test]
    fn test_budget_soft_limit_warning() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        // Set budget to max 5 tiles; soft limit at 80% = 4 tiles
        scene.leases.get_mut(&lease_id).unwrap().resource_budget.max_tiles = 5;

        // Create 4 tiles (should trigger soft limit warning on the 4th)
        for i in 0..4 {
            let batch = crate::mutation::MutationBatch {
                batch_id: SceneId::new(),
                agent_namespace: "test".to_string(),
                mutations: vec![crate::mutation::SceneMutation::CreateTile {
                    tab_id,
                    namespace: "test".to_string(),
                    lease_id,
                    bounds: Rect::new(i as f32 * 120.0, 0.0, 100.0, 100.0),
                    z_order: i + 1,
                }],
                timing_hints: None,
                lease_id: None,
            };
            scene.apply_batch(&batch);
        }

        assert!(scene.is_lease_budget_warning(&lease_id));

        // 5th tile should succeed (within hard limit) but with budget_warning
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(480.0, 0.0, 100.0, 100.0),
                z_order: 5,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert!(result.budget_warning);
    }

    // ─── Suspension Tests ───────────────────────────────────────────────

    #[test]
    fn test_suspend_blocks_mutations() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        // Suspend the lease
        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Try to create a tile — should fail
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(!result.applied);
        assert_eq!(scene.tile_count(), 0);
    }

    #[test]
    fn test_resume_allows_mutations_again() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        // Suspend then resume
        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();
        clock.advance(2_000);
        scene.resume_lease(&lease_id, clock.now_millis()).unwrap();

        // Create a tile — should succeed
        let batch = crate::mutation::MutationBatch {
            batch_id: SceneId::new(),
            agent_namespace: "test".to_string(),
            mutations: vec![crate::mutation::SceneMutation::CreateTile {
                tab_id,
                namespace: "test".to_string(),
                lease_id,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                z_order: 1,
            }],
            timing_hints: None,
            lease_id: None,
        };
        let result = scene.apply_batch(&batch);
        assert!(result.applied);
        assert_eq!(scene.tile_count(), 1);
    }

    #[test]
    fn test_ttl_paused_during_suspension() {
        let (mut scene, clock) = scene_with_test_clock();
        // Grant a 10-second lease
        let lease_id = scene.grant_lease("test", 10_000, vec![]);

        // At t=5s, suspend
        clock.advance(5_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();
        let remaining_at_suspend = scene.leases[&lease_id].ttl_remaining_at_suspend_ms;
        assert_eq!(remaining_at_suspend, Some(5_000));

        // Advance 20 seconds while suspended
        clock.advance(20_000);
        // Should NOT be expired (TTL paused)
        assert!(!scene.leases[&lease_id].is_expired(clock.now_millis()));
        // Remaining should still be 5_000
        assert_eq!(scene.leases[&lease_id].remaining_ms(clock.now_millis()), 5_000);

        // Resume
        scene.resume_lease(&lease_id, clock.now_millis()).unwrap();
        // Now remaining should be 5_000 from the resume point
        assert_eq!(scene.leases[&lease_id].remaining_ms(clock.now_millis()), 5_000);

        // Advance 4 seconds — not yet expired
        clock.advance(4_000);
        assert!(!scene.leases[&lease_id].is_expired(clock.now_millis()));
        assert_eq!(scene.leases[&lease_id].remaining_ms(clock.now_millis()), 1_000);

        // Advance 2 more seconds — now expired
        clock.advance(2_000);
        assert!(scene.leases[&lease_id].is_expired(clock.now_millis()));
    }

    // ─── Grace Period Tests ─────────────────────────────────────────────

    #[test]
    fn test_grace_period_disconnect_and_reconnect() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 120_000, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        // Disconnect
        clock.advance(5_000);
        scene.disconnect_lease(&lease_id, clock.now_millis()).unwrap();
        assert_eq!(scene.tile_count(), 1); // Tiles preserved

        // Reconnect within grace (30s)
        clock.advance(15_000);
        scene.reconnect_lease(&lease_id, clock.now_millis()).unwrap();
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Active);
        assert_eq!(scene.tile_count(), 1); // Tiles still there
    }

    #[test]
    fn test_grace_period_expiry_cleans_up() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 120_000, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        // Disconnect
        clock.advance(5_000);
        scene.disconnect_lease(&lease_id, clock.now_millis()).unwrap();

        // Grace period expires (30s)
        clock.advance(31_000);
        let expiries = scene.expire_leases();
        assert_eq!(expiries.len(), 1);
        assert_eq!(expiries[0].terminal_state, LeaseState::Expired);
        assert_eq!(scene.tile_count(), 0); // Tiles cleaned up
    }

    #[test]
    fn test_grace_period_check() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 120_000, vec![]);

        clock.advance(5_000);
        scene.disconnect_lease(&lease_id, clock.now_millis()).unwrap();

        // Not expired yet
        clock.advance(29_000);
        assert!(!scene.leases[&lease_id].check_grace_expired(clock.now_millis()));

        // Expired
        clock.advance(2_000);
        assert!(scene.leases[&lease_id].check_grace_expired(clock.now_millis()));
    }

    // ─── Safe Mode Tests ────────────────────────────────────────────────

    #[test]
    fn test_suspend_all_leases() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);
        let l3 = scene.grant_lease("agent3", 60_000, vec![]);

        clock.advance(5_000);
        scene.suspend_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Suspended);
        assert_eq!(scene.leases[&l2].state, LeaseState::Suspended);
        assert_eq!(scene.leases[&l3].state, LeaseState::Suspended);
    }

    #[test]
    fn test_resume_all_leases() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);

        clock.advance(5_000);
        scene.suspend_all_leases(clock.now_millis());

        clock.advance(2_000);
        scene.resume_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Active);
        assert_eq!(scene.leases[&l2].state, LeaseState::Active);
    }

    #[test]
    fn test_suspend_all_skips_non_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);

        // Disconnect l2 first
        clock.advance(1_000);
        scene.disconnect_lease(&l2, clock.now_millis()).unwrap();

        // Suspend all — only l1 should be suspended
        clock.advance(1_000);
        scene.suspend_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Suspended);
        assert_eq!(scene.leases[&l2].state, LeaseState::Orphaned); // Unchanged (was Disconnected)
    }

    #[test]
    fn test_resume_all_only_resumes_suspended() {
        let (mut scene, clock) = scene_with_test_clock();
        let l1 = scene.grant_lease("agent1", 60_000, vec![]);
        let l2 = scene.grant_lease("agent2", 60_000, vec![]);

        // Disconnect l2
        clock.advance(1_000);
        scene.disconnect_lease(&l2, clock.now_millis()).unwrap();

        // Suspend only l1
        clock.advance(1_000);
        scene.suspend_lease(&l1, clock.now_millis()).unwrap();

        // Resume all — only l1 should be resumed
        clock.advance(1_000);
        scene.resume_all_leases(clock.now_millis());

        assert_eq!(scene.leases[&l1].state, LeaseState::Active);
        assert_eq!(scene.leases[&l2].state, LeaseState::Orphaned); // Unchanged (was Disconnected)
    }

    #[test]
    fn test_suspension_timeout_revokes() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 600_000, vec![Capability::CreateTile]);
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Use a short max_suspend for testing
        let max_suspend = 5_000;
        clock.advance(6_000);

        let expiries = scene.expire_leases_with_max_suspend(max_suspend);
        assert_eq!(expiries.len(), 1);
        assert_eq!(expiries[0].terminal_state, LeaseState::Revoked);
        assert_eq!(scene.leases[&lease_id].state, LeaseState::Revoked);
        assert_eq!(scene.tile_count(), 0);
    }

    // ─── Renewal Policy Tests ───────────────────────────────────────────

    #[test]
    fn test_renewal_policy_defaults_to_manual() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        assert_eq!(scene.leases[&lease_id].renewal_policy, RenewalPolicy::Manual);
    }

    #[test]
    fn test_lease_priority_defaults_to_normal() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease("test", 60_000, vec![]);
        assert_eq!(scene.leases[&lease_id].priority, 2);
    }

    // ─── Priority Persistence Tests ─────────────────────────────────────
    // Spec §Requirement: Priority Assignment (lease-governance/spec.md lines 49-60)
    // Spec §Requirement: Priority Sort Semantics (lease-governance/spec.md lines 62-69)

    /// WHEN grant_lease_with_priority is called with priority 1
    /// THEN the persisted lease priority is 1.
    ///
    /// Validates that the scene graph stores the effective priority verbatim so the
    /// degradation ladder can sort tiles by (lease_priority ASC, z_order DESC) without
    /// consulting the session layer.
    #[test]
    fn test_grant_lease_with_priority_persists_value() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_high = scene.grant_lease_with_priority("agent-high", 60_000, 1, vec![]);
        let lease_normal = scene.grant_lease_with_priority("agent-normal", 60_000, 2, vec![]);
        let lease_low = scene.grant_lease_with_priority("agent-low", 60_000, 3, vec![]);

        assert_eq!(scene.leases[&lease_high].priority, 1, "high priority must be stored as 1");
        assert_eq!(scene.leases[&lease_normal].priority, 2, "normal priority must be stored as 2");
        assert_eq!(scene.leases[&lease_low].priority, 3, "low priority must be stored as 3");
    }

    /// WHEN a lease is renewed THEN the stored priority is preserved unchanged.
    ///
    /// Spec: renewal updates the TTL clock but must not change the effective priority.
    #[test]
    fn test_renew_lease_preserves_priority() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let lease_id = scene.grant_lease_with_priority("agent", 60_000, 1, vec![]);

        // Verify priority before renewal.
        assert_eq!(scene.leases[&lease_id].priority, 1);

        // Renew the lease with a new TTL.
        scene.renew_lease(lease_id, 120_000).expect("renewal must succeed");

        // Priority must remain unchanged after renewal.
        assert_eq!(
            scene.leases[&lease_id].priority, 1,
            "priority must be preserved across renewal"
        );
        // TTL must be updated.
        assert_eq!(scene.leases[&lease_id].ttl_ms, 120_000);
    }

    /// WHEN multiple leases are granted with distinct priorities
    /// THEN the degradation ladder shedding order is (priority DESC numerically, z_order ASC).
    ///
    /// Spec §Requirement: Tile Shedding Order (runtime-kernel/spec.md lines 263-270):
    /// tiles with the highest lease_priority values (least important) shed first.
    #[test]
    fn test_grant_lease_with_priority_shedding_order() {
        use crate::lease::priority::{shed_count_for_level4, shedding_order, TileSheddingEntry};

        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let _l_high = scene.grant_lease_with_priority("chrome", 60_000, 0, vec![]);
        let _l_normal = scene.grant_lease_with_priority("agent-normal", 60_000, 2, vec![]);
        let _l_low = scene.grant_lease_with_priority("agent-low", 60_000, 3, vec![]);

        // Build TileSheddingEntry list using the stored priorities.
        // (In production the runtime reads l.priority directly from the lease record.)
        let entries: Vec<TileSheddingEntry> = scene
            .leases
            .values()
            .enumerate()
            .map(|(i, l)| TileSheddingEntry::new(i, l.priority, 5))
            .collect();

        let count = shed_count_for_level4(entries.len());
        let shed = shedding_order(&entries, count);

        // The shed entry must be the lease with the highest priority value (priority=3).
        let shed_priorities: Vec<u8> = shed
            .iter()
            .map(|&i| entries[i].key.lease_priority)
            .collect();
        assert!(
            shed_priorities.iter().all(|&p| p == 3),
            "only the lowest-priority (highest value) lease should shed first; got {:?}",
            shed_priorities
        );
    }

    // ─── Resource Usage Tests ───────────────────────────────────────────

    #[test]
    fn test_lease_resource_usage() {
        let mut scene = SceneGraph::new(1920.0, 1080.0);
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 60_000, vec![Capability::CreateTile]);

        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();
        scene
            .create_tile(tab_id, "test", lease_id, Rect::new(200.0, 0.0, 100.0, 100.0), 2)
            .unwrap();

        let usage = scene.lease_resource_usage(&lease_id);
        assert_eq!(usage.tiles, 2);
    }

    #[test]
    fn test_renew_lease_fails_when_not_active() {
        let (mut scene, clock) = scene_with_test_clock();
        let lease_id = scene.grant_lease("test", 60_000, vec![]);

        // Suspend lease
        clock.advance(1_000);
        scene.suspend_lease(&lease_id, clock.now_millis()).unwrap();

        // Renew should fail (lease not active)
        let err = scene.renew_lease(lease_id, 120_000);
        assert!(err.is_err());
    }

    #[test]
    fn test_lease_expiry_returns_lease_expiry_struct() {
        let (mut scene, clock) = scene_with_test_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("test", 500, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(tab_id, "test", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        clock.advance(501);
        let expiries = scene.expire_leases();
        assert_eq!(expiries.len(), 1);
        assert_eq!(expiries[0].lease_id, lease_id);
        assert_eq!(expiries[0].terminal_state, LeaseState::Expired);
        assert!(expiries[0].removed_tiles.contains(&tile_id));
    }
}

// ─── Spec scenario tests (RFC 0001 §2.1–§2.4) ────────────────────────────────
//
// Each test corresponds to a WHEN/THEN scenario from the issue spec.

#[cfg(test)]
mod spec_scenarios {
    use super::*;
    use crate::clock::TestClock;
    use crate::types::{
        Capability, Node, NodeData, Rect, Rgba, SceneId, SolidColorNode,
        TextMarkdownNode, FontFamily, TextAlign, TextOverflow, HitRegionNode,
    };
    use std::sync::Arc;

    fn make_scene() -> SceneGraph {
        SceneGraph::new(1920.0, 1080.0)
    }

    fn make_scene_with_clock() -> (SceneGraph, Arc<TestClock>) {
        let clock = Arc::new(TestClock::new(1_000_000));
        let scene = SceneGraph::new_with_clock(1920.0, 1080.0, clock.clone());
        (scene, clock)
    }

    // ─ Tab limit enforcement (spec line 50) ──────────────────────────────────
    // WHEN an agent attempts CreateTab and 256 tabs already exist
    // THEN the runtime MUST reject with BudgetExceeded

    #[test]
    fn tab_limit_256_enforced() {
        let mut scene = make_scene();
        for i in 0..MAX_TABS {
            scene.create_tab(&format!("Tab {}", i), i as u32).expect("should create tab");
        }
        assert_eq!(scene.tabs.len(), MAX_TABS);
        let err = scene.create_tab("Overflow", MAX_TABS as u32).unwrap_err();
        assert!(
            matches!(err, ValidationError::BudgetExceeded { .. }),
            "expected BudgetExceeded, got {:?}",
            err
        );
    }

    // ─ Tile limit enforcement (spec line 54) ─────────────────────────────────
    // WHEN an agent attempts CreateTile on a tab that already has 1024 tiles
    // THEN the runtime MUST reject with BudgetExceeded

    #[test]
    fn tile_limit_1024_per_tab_enforced() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTile]);

        // The test scene is 1920×1080; tiles are 1px×1px at unique positions.
        // Use a grid: 32 cols × 32 rows = 1024. We'll use tiny tiles in bounds.
        // Actually: MAX_TILES_PER_TAB = 1024.
        for i in 0..(MAX_TILES_PER_TAB) {
            let x = (i % 40) as f32 * 48.0;
            let y = (i / 40) as f32 * 42.0;
            if x + 40.0 <= 1920.0 && y + 40.0 <= 1080.0 {
                scene
                    .create_tile(tab_id, "agent", lease_id, Rect::new(x, y, 40.0, 40.0), i as u32)
                    .expect("should create tile within limit");
            } else {
                // Re-use same position for tiles that would go out of bounds (unchecked path ignores bounds)
                scene
                    .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 1.0, 1.0), i as u32)
                    .expect("should create tile within limit");
            }
        }
        assert_eq!(
            scene.tiles.values().filter(|t| t.tab_id == tab_id).count(),
            MAX_TILES_PER_TAB
        );

        let err = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 1.0, 1.0), MAX_TILES_PER_TAB as u32)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::BudgetExceeded { .. }),
            "expected BudgetExceeded, got {:?}",
            err
        );
    }

    // ─ Node limit enforcement (spec line 58) ─────────────────────────────────
    // WHEN an agent attempts InsertNode on a tile with 64 nodes
    // THEN the runtime MUST reject with NodeCountExceeded

    #[test]
    fn node_limit_64_per_tile_enforced() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTile, Capability::CreateNode]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 400.0, 400.0), 1)
            .unwrap();

        // Add root node first, then chain children off the root.
        let root_id = SceneId::new();
        let root_node = Node {
            id: root_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 400.0, 400.0),
            }),
        };
        scene.add_node_to_tile(tile_id, None, root_node).expect("root should be added");

        // Add MAX_NODES_PER_TILE - 1 children off the root (total will be MAX_NODES_PER_TILE)
        for i in 1..MAX_NODES_PER_TILE {
            let child = Node {
                id: SceneId::new(),
                children: vec![],
                data: NodeData::SolidColor(SolidColorNode {
                    color: Rgba::new(0.1 * (i % 10) as f32, 0.0, 0.0, 1.0),
                    bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
                }),
            };
            scene.add_node_to_tile(tile_id, Some(root_id), child)
                .unwrap_or_else(|e| panic!("should add child {} ok: {:?}", i, e));
        }

        // Verify we have exactly MAX_NODES_PER_TILE nodes in the tile
        let count = scene.count_node_subtree(root_id);
        assert_eq!(count as usize, MAX_NODES_PER_TILE, "should have exactly {} nodes", MAX_NODES_PER_TILE);

        // One more should be rejected
        let overflow_node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::BLACK,
                bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
            }),
        };
        let err = scene.add_node_to_tile(tile_id, Some(root_id), overflow_node).unwrap_err();
        assert!(
            matches!(err, ValidationError::NodeCountExceeded { .. }),
            "expected NodeCountExceeded, got {:?}",
            err
        );
    }

    // ─ Duplicate NodeId rejection (spec line 62) ─────────────────────────────
    // WHEN an agent attempts to add a node with a NodeId that already exists in the scene
    // THEN the runtime MUST reject with DuplicateId

    #[test]
    fn duplicate_node_id_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTile]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 200.0, 200.0), 1)
            .unwrap();

        let node_id = SceneId::new();
        let node = Node {
            id: node_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            }),
        };
        // First insertion succeeds
        scene.add_node_to_tile(tile_id, None, node.clone()).expect("first insert should succeed");

        // Second insertion with the same node ID should fail
        let tile_id2 = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(200.0, 0.0, 200.0, 200.0), 2)
            .unwrap();
        let err = scene.add_node_to_tile(tile_id2, None, node).unwrap_err();
        assert!(
            matches!(err, ValidationError::DuplicateId { id } if id == node_id),
            "expected DuplicateId, got {:?}",
            err
        );
    }

    // ─ Tab name too long (spec line 79) ──────────────────────────────────────
    // WHEN an agent submits CreateTab with a name exceeding 128 UTF-8 bytes
    // THEN the runtime MUST reject with InvalidFieldValue

    #[test]
    fn tab_name_too_long_rejected() {
        let mut scene = make_scene();
        let long_name = "a".repeat(MAX_TAB_NAME_BYTES + 1);
        let err = scene.create_tab(&long_name, 0).unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "name"),
            "expected InvalidField for name, got {:?}",
            err
        );
    }

    // ─ Tab mutation without capability (spec line 83) ─────────────────────────
    // WHEN an agent without manage_tabs capability submits CreateTab
    // THEN the runtime MUST reject with CapabilityMissing

    #[test]
    fn tab_create_without_manage_tabs_rejected() {
        let mut scene = make_scene();
        // Lease with no capabilities
        let lease_id = scene.grant_lease("agent", 300_000, vec![]);
        let err = scene.create_tab_with_lease("My Tab", 0, lease_id).unwrap_err();
        assert!(
            matches!(err, ValidationError::CapabilityMissing { ref capability } if capability.contains("ManageTabs")),
            "expected CapabilityMissing(ManageTabs), got {:?}",
            err
        );
    }

    // ─ Create and switch tab (spec line 71) ──────────────────────────────────
    // WHEN an agent with manage_tabs submits CreateTab + SwitchActiveTab
    // THEN the new tab MUST be created and become active

    #[test]
    fn create_and_switch_tab_with_capability() {
        let mut scene = make_scene();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::ManageTabs]);
        let tab_id = scene.create_tab_with_lease("New Tab", 0, lease_id).unwrap();
        scene.switch_active_tab_with_lease(tab_id, lease_id).unwrap();
        assert_eq!(scene.active_tab, Some(tab_id));
    }

    // ─ Tab rename (spec line 75) ─────────────────────────────────────────────
    // WHEN an agent submits RenameTab with a new name of 100 UTF-8 bytes
    // THEN the tab name MUST be updated

    #[test]
    fn rename_tab_with_100_byte_name() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Original", 0).unwrap();
        let new_name = "a".repeat(100);
        scene.rename_tab(tab_id, &new_name).unwrap();
        assert_eq!(scene.tabs[&tab_id].name, new_name);
    }

    // ─ Create tile with valid lease (spec line 92) ────────────────────────────
    // WHEN an agent with create_tiles + modify_own_tiles and valid lease submits CreateTile
    // THEN the tile MUST be created with specified bounds, z_order, and opacity

    #[test]
    fn create_tile_checked_requires_capabilities() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // No capabilities — should fail
        let lease_no_caps = scene.grant_lease("agent", 300_000, vec![]);
        let err = scene
            .create_tile_checked(tab_id, "agent", lease_no_caps, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap_err();
        assert!(matches!(err, ValidationError::CapabilityMissing { .. }), "got {:?}", err);

        // Only create_tiles (not modify_own_tiles) — should still fail
        let lease_create_only = scene.grant_lease("agent", 300_000, vec![Capability::CreateTiles]);
        let err = scene
            .create_tile_checked(tab_id, "agent", lease_create_only, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap_err();
        assert!(matches!(err, ValidationError::CapabilityMissing { .. }), "got {:?}", err);

        // Full capabilities — should succeed
        let lease_full = scene.grant_lease(
            "agent",
            300_000,
            vec![Capability::CreateTiles, Capability::ModifyOwnTiles],
        );
        let tile_id = scene
            .create_tile_checked(tab_id, "agent", lease_full, Rect::new(0.0, 0.0, 200.0, 200.0), 5)
            .unwrap();
        assert_eq!(scene.tiles[&tile_id].z_order, 5);
        assert!((scene.tiles[&tile_id].opacity - 1.0).abs() < f32::EPSILON);
    }

    // ─ Tile mutation with expired lease (spec line 96) ───────────────────────
    // WHEN an agent submits UpdateTileBounds but the tile's lease has expired
    // THEN the runtime MUST reject with LeaseExpired

    #[test]
    fn tile_mutation_with_expired_lease_rejected() {
        let (mut scene, clock) = make_scene_with_clock();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 100, vec![Capability::CreateTiles, Capability::ModifyOwnTiles]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 200.0, 200.0), 1)
            .unwrap();

        // Advance clock past TTL
        clock.advance(200);

        let err = scene
            .update_tile_bounds(tile_id, Rect::new(10.0, 10.0, 100.0, 100.0), "agent")
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::LeaseExpired { .. }),
            "expected LeaseExpired, got {:?}",
            err
        );
    }

    // ─ Delete tile (spec line 100) ─────────────────────────────────────────────
    // WHEN an agent submits DeleteTile for a tile it owns with a valid lease
    // THEN the tile and all its nodes MUST be removed

    #[test]
    fn delete_tile_removes_tile_and_nodes() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTiles, Capability::ModifyOwnTiles]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 200.0, 200.0), 1)
            .unwrap();
        let node_id = SceneId::new();
        scene.set_tile_root(tile_id, Node {
            id: node_id,
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::WHITE,
                bounds: Rect::new(0.0, 0.0, 200.0, 200.0),
            }),
        }).unwrap();
        assert!(scene.nodes.contains_key(&node_id));

        scene.delete_tile(tile_id, "agent").unwrap();
        assert!(!scene.tiles.contains_key(&tile_id), "tile should be removed");
        assert!(!scene.nodes.contains_key(&node_id), "nodes should be removed with tile");
    }

    // ─ Opacity out of range (spec line 109) ──────────────────────────────────
    // WHEN an agent submits UpdateTileOpacity with opacity = 1.5
    // THEN the runtime MUST reject with InvalidFieldValue

    #[test]
    fn opacity_out_of_range_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTiles, Capability::ModifyOwnTiles]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 200.0, 200.0), 1)
            .unwrap();

        let err = scene.update_tile_opacity(tile_id, 1.5, "agent").unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "opacity"),
            "expected InvalidField(opacity), got {:?}",
            err
        );

        let err2 = scene.update_tile_opacity(tile_id, -0.1, "agent").unwrap_err();
        assert!(matches!(err2, ValidationError::InvalidField { .. }), "got {:?}", err2);
    }

    // ─ Zero-size bounds (spec line 113) ──────────────────────────────────────
    // WHEN an agent submits CreateTile with width = 0.0
    // THEN the runtime MUST reject with BoundsOutOfRange

    #[test]
    fn zero_size_bounds_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();

        // create_tile_checked requires CreateTiles + ModifyOwnTiles; use correct capabilities
        // so the bounds check is reached (not capability check).
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTiles, Capability::ModifyOwnTiles]);

        let err = scene
            .create_tile_checked(
                tab_id,
                "agent",
                lease_id,
                Rect::new(0.0, 0.0, 0.0, 100.0), // width = 0.0
                1,
            )
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::BoundsOutOfRange { .. }),
            "expected BoundsOutOfRange, got {:?}", err
        );

        // Use the basic create_tile (no capability check) to also confirm bounds are rejected
        let lease_unchecked = scene.grant_lease("agent", 300_000, vec![Capability::CreateTile]);
        let err2 = scene
            .create_tile(tab_id, "agent", lease_unchecked, Rect::new(0.0, 0.0, 0.0, 100.0), 1)
            .unwrap_err();
        assert!(
            matches!(err2, ValidationError::BoundsOutOfRange { .. }),
            "expected BoundsOutOfRange, got {:?}",
            err2
        );
    }

    // ─ Bounds outside tab area (spec line 117) ───────────────────────────────
    // WHEN UpdateTileBounds with x + width exceeding tab display width
    // THEN reject with BoundsOutOfRange

    #[test]
    fn bounds_outside_display_rejected() {
        let mut scene = make_scene(); // 1920×1080
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTile]);

        let err = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(1800.0, 0.0, 200.0, 100.0), 1) // x + w = 2000 > 1920
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::BoundsOutOfRange { .. }),
            "expected BoundsOutOfRange, got {:?}",
            err
        );
    }

    // ─ Z-order in reserved zone band (spec line 121) ─────────────────────────
    // WHEN CreateTile with z_order = ZONE_TILE_Z_MIN
    // THEN reject with InvalidFieldValue

    #[test]
    fn z_order_reserved_zone_band_rejected() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTile]);

        let err = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), ZONE_TILE_Z_MIN)
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::InvalidField { ref field, .. } if field == "z_order"),
            "expected InvalidField(z_order), got {:?}",
            err
        );

        // Also reject z_order above the threshold
        let err2 = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), ZONE_TILE_Z_MIN + 1)
            .unwrap_err();
        assert!(matches!(err2, ValidationError::InvalidField { .. }), "got {:?}", err2);

        // z_order just below threshold is fine
        scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), ZONE_TILE_Z_MIN - 1)
            .expect("z_order just below ZONE_TILE_Z_MIN must succeed");
    }

    // ─ TextMarkdownNode content limit (spec line 130) ─────────────────────────
    // WHEN TextMarkdownNode with content exceeding 65535 UTF-8 bytes
    // THEN reject with InvalidFieldValue

    #[test]
    fn text_markdown_content_limit_enforced() {
        let oversized = "x".repeat(MAX_MARKDOWN_BYTES + 1);
        // Validate that the node construction itself is possible but the validation
        // catches it. We check via validate_node_data if it exists, or directly.
        // For now, test that creating such content is flagged at the graph level.
        let node = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: oversized.clone(),
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: None,
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };
        // The validation function
        let err = validate_text_markdown_node_data(&node.data);
        assert!(err.is_some(), "oversized content should be flagged");
    }

    // ─ Cross-namespace tile access denied (spec line 37) ─────────────────────
    // WHEN agent "weather-agent" attempts to mutate a tile owned by namespace "cal"
    // THEN reject with CapabilityMissing or LeaseNotFound

    #[test]
    fn cross_namespace_tile_access_denied() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let cal_lease = scene.grant_lease("cal", 300_000, vec![Capability::CreateTiles, Capability::ModifyOwnTiles]);
        let tile_id = scene
            .create_tile(tab_id, "cal", cal_lease, Rect::new(0.0, 0.0, 200.0, 200.0), 1)
            .unwrap();

        // weather-agent tries to update bounds of cal's tile
        let err = scene
            .update_tile_bounds(tile_id, Rect::new(10.0, 10.0, 100.0, 100.0), "wtr")
            .unwrap_err();
        assert!(
            matches!(err, ValidationError::NamespaceMismatch { .. }),
            "expected NamespaceMismatch, got {:?}",
            err
        );
    }

    // ─ Struct size budgets (spec line 307, 311) ───────────────────────────────
    // Tile < 200 bytes, Node < 150 bytes

    #[test]
    fn tile_struct_size_under_200_bytes() {
        use std::mem::size_of;
        let tile_size = size_of::<Tile>();
        assert!(
            tile_size < 200,
            "Tile struct is {} bytes, must be < 200 bytes per RFC 0001 §8",
            tile_size
        );
    }

    #[test]
    fn node_struct_size_under_150_bytes() {
        use std::mem::size_of;
        let node_size = size_of::<Node>();
        assert!(
            node_size < 150,
            "Node struct is {} bytes, must be < 150 bytes per RFC 0001 §8",
            node_size
        );
    }

    // ─ Tab CRUD full cycle ────────────────────────────────────────────────────

    #[test]
    fn tab_delete_removes_tiles_too() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTile]);
        scene.create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1).unwrap();
        assert_eq!(scene.tile_count(), 1);

        scene.delete_tab(tab_id).unwrap();
        assert_eq!(scene.tabs.len(), 0, "tab should be removed");
        assert_eq!(scene.tile_count(), 0, "tiles should be removed with tab");
        assert_eq!(scene.active_tab, None, "active_tab should be None after deleting last tab");
    }

    #[test]
    fn tab_reorder_updates_display_order() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        scene.reorder_tab(tab_id, 5).unwrap();
        assert_eq!(scene.tabs[&tab_id].display_order, 5);
    }

    #[test]
    fn tab_reorder_conflict_rejected() {
        let mut scene = make_scene();
        let tab_a = scene.create_tab("A", 0).unwrap();
        let _tab_b = scene.create_tab("B", 1).unwrap();
        // Try to give tab_a the same order as tab_b
        let err = scene.reorder_tab(tab_a, 1).unwrap_err();
        assert!(matches!(err, ValidationError::DuplicateDisplayOrder { .. }), "got {:?}", err);
    }

    // ─ Opacity valid range ────────────────────────────────────────────────────

    #[test]
    fn tile_opacity_accepts_boundary_values() {
        let mut scene = make_scene();
        let tab_id = scene.create_tab("Main", 0).unwrap();
        let lease_id = scene.grant_lease("agent", 300_000, vec![Capability::CreateTiles, Capability::ModifyOwnTiles]);
        let tile_id = scene
            .create_tile(tab_id, "agent", lease_id, Rect::new(0.0, 0.0, 100.0, 100.0), 1)
            .unwrap();

        scene.update_tile_opacity(tile_id, 0.0, "agent").unwrap();
        assert!((scene.tiles[&tile_id].opacity - 0.0).abs() < f32::EPSILON);

        scene.update_tile_opacity(tile_id, 1.0, "agent").unwrap();
        assert!((scene.tiles[&tile_id].opacity - 1.0).abs() < f32::EPSILON);

        scene.update_tile_opacity(tile_id, 0.5, "agent").unwrap();
        assert!((scene.tiles[&tile_id].opacity - 0.5).abs() < f32::EPSILON);
    }

    // ─ All 25 test scenes pass Layer 0 invariants ────────────────────────────

    #[test]
    fn all_25_test_scenes_pass_layer0_invariants() {
        use crate::test_scenes::{assert_layer0_invariants, ClockMs, TestSceneRegistry};

        let registry = TestSceneRegistry::new();
        let names = TestSceneRegistry::scene_names();
        assert_eq!(names.len(), 25, "must have exactly 25 registered scenes, got {}", names.len());

        for name in names {
            let (graph, _spec) = registry
                .build(name, ClockMs::FIXED)
                .unwrap_or_else(|| panic!("scene '{}' failed to build", name));
            let violations = assert_layer0_invariants(&graph);
            assert!(
                violations.is_empty(),
                "scene '{}' has Layer 0 violations: {:?}",
                name,
                violations
            );
        }
    }

    // ─ V1 node types constructable without GPU ───────────────────────────────

    #[test]
    fn all_v1_node_types_constructable() {
        // SolidColorNode
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::SolidColor(SolidColorNode {
                color: Rgba::new(0.5, 0.5, 0.5, 1.0),
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            }),
        };

        // TextMarkdownNode
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::TextMarkdown(TextMarkdownNode {
                content: "# Hello".to_string(),
                bounds: Rect::new(0.0, 0.0, 400.0, 200.0),
                font_size_px: 16.0,
                font_family: FontFamily::SystemSansSerif,
                color: Rgba::WHITE,
                background: None,
                alignment: TextAlign::Start,
                overflow: TextOverflow::Clip,
            }),
        };

        // HitRegionNode
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::HitRegion(HitRegionNode {
                bounds: Rect::new(10.0, 10.0, 100.0, 50.0),
                interaction_id: "btn-ok".to_string(),
                accepts_focus: true,
                accepts_pointer: true,
                ..Default::default()
            }),
        };

        // StaticImageNode — constructable without GPU context
        // RS-4: uses resource_id + decoded_bytes, no raw blob data embedded.
        use crate::types::StaticImageNode;
        use crate::types::ImageFitMode;
        let _ = Node {
            id: SceneId::new(),
            children: vec![],
            data: NodeData::StaticImage(StaticImageNode {
                resource_id: ResourceId::of(b"4x4 test image"),
                width: 4,
                height: 4,
                decoded_bytes: 4u64 * 4 * 4, // 4×4 RGBA8
                fit_mode: ImageFitMode::Contain,
                bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            }),
        };
    }
}

// ─── Helper for TextMarkdownNode content size validation ──────────────────────

/// Validate a TextMarkdownNode's content size.
///
/// Returns `Some(ValidationError)` if the content exceeds `MAX_MARKDOWN_BYTES`.
/// Used by `set_tile_root_impl` when strict content validation is needed.
pub fn validate_text_markdown_node_data(data: &NodeData) -> Option<ValidationError> {
    if let NodeData::TextMarkdown(tm) = data {
        if tm.content.len() > MAX_MARKDOWN_BYTES {
            return Some(ValidationError::InvalidField {
                field: "content".into(),
                reason: format!(
                    "TextMarkdownNode content exceeds {} UTF-8 bytes (got {})",
                    MAX_MARKDOWN_BYTES,
                    tm.content.len()
                ),
            });
        }
    }
    None
}
