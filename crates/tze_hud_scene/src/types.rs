//! Core types for the scene graph, following RFC 0001.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── IDs ────────────────────────────────────────────────────────────────────

/// Scene object ID — UUIDv7 (time-ordered, 16 bytes).
///
/// # Wire format
/// Serialized as 16 raw bytes in little-endian UUID byte order (as returned by
/// [`Uuid::to_bytes_le`]). The all-zero value (`[0u8; 16]`) is the null/absent
/// sentinel per RFC 0001 §1.1.
///
/// # Invariants
/// - `size_of::<SceneId>() == 16`
/// - Lexicographic sort order == creation-time order (UUIDv7 property)
/// - `SceneId::null().is_null() == true`
/// - `SceneId::new().is_null() == false` (freshly-generated IDs are never null)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SceneId(Uuid);

impl SceneId {
    pub fn new() -> Self {
        SceneId(Uuid::now_v7())
    }

    /// Create from raw UUID (for testing / deserialization).
    pub fn from_uuid(uuid: Uuid) -> Self {
        SceneId(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Null/zero ID used as the "absent" sentinel (RFC 0001 §1.1).
    ///
    /// Protobuf fields absent their presence bit decode to this value.
    pub fn null() -> Self {
        SceneId(Uuid::nil())
    }

    /// Returns `true` if this is the null/absent sentinel (`[0u8; 16]`).
    pub fn is_null(&self) -> bool {
        self.0.is_nil()
    }

    /// Nil/zero ID used as "none" sentinel in protobuf.
    ///
    /// Alias for [`Self::null`]; prefer `null()`/`is_null()` in new code.
    #[inline]
    pub fn nil() -> Self {
        Self::null()
    }

    /// Returns `true` if this is the nil/zero sentinel.
    ///
    /// Alias for [`Self::is_null`]; prefer `is_null()` in new code.
    #[inline]
    pub fn is_nil(&self) -> bool {
        self.is_null()
    }

    /// Serialize to 16 bytes in little-endian UUID byte order.
    ///
    /// Used for protobuf `bytes` fields. The encoding is stable and matches
    /// the wire contract from RFC 0001 §4.1.
    pub fn to_bytes_le(&self) -> [u8; 16] {
        self.0.to_bytes_le()
    }

    /// Deserialize from 16 bytes in little-endian UUID byte order.
    ///
    /// Returns `None` if the slice is not exactly 16 bytes.
    pub fn from_bytes_le(bytes: &[u8]) -> Option<Self> {
        let arr: [u8; 16] = bytes.try_into().ok()?;
        Some(SceneId(Uuid::from_bytes_le(arr)))
    }
}

impl Default for SceneId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SceneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ─── ResourceId ──────────────────────────────────────────────────────────────

/// Content-addressed resource identity — 32-byte BLAKE3 hash.
///
/// Two agents uploading identical content MUST receive the same `ResourceId`;
/// the runtime stores the resource once (RFC 0001 §1.1).
///
/// # Wire format
/// Stored and transmitted as raw 32 bytes. Hex encoding is a display/debug
/// concern only and MUST NOT appear on the wire or in storage.
///
/// # Invariants
/// - `size_of::<ResourceId>() == 32`
/// - Equality is byte equality — no normalisation
/// - `ResourceId::of(bytes) == ResourceId::of(same_bytes)` always
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ResourceId([u8; 32]);

impl ResourceId {
    /// Compute the `ResourceId` for a byte payload using BLAKE3.
    pub fn of(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        ResourceId(*hash.as_bytes())
    }

    /// Wrap a raw 32-byte array directly (for deserialization / testing).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        ResourceId(bytes)
    }

    /// Return the raw 32-byte hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Try to construct from a byte slice.
    ///
    /// Returns `None` if the slice is not exactly 32 bytes.
    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        let arr: [u8; 32] = slice.try_into().ok()?;
        Some(ResourceId(arr))
    }

    /// Return a lowercase hex string for display / logging only.
    ///
    /// MUST NOT be used on the wire or in storage.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

// ─── Geometry ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains_point(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.width && py >= self.y && py < self.y + self.height
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    /// Check if this rect is fully contained within `outer`.
    pub fn is_within(&self, outer: &Rect) -> bool {
        self.x >= outer.x
            && self.y >= outer.y
            && self.x + self.width <= outer.x + outer.width
            && self.y + self.height <= outer.y + outer.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Convert to [f32; 4] for GPU upload.
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

// ─── Enums ──────────────────────────────────────────────────────────────────

/// How image content is fitted within the node's bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFitMode {
    /// Scale uniformly so the entire image is visible; may leave letterbox bars.
    Contain,
    /// Scale uniformly to cover the entire bounds; may crop the image.
    Cover,
    /// Stretch non-uniformly to fill bounds exactly.
    Fill,
    /// Like Contain but never scale up; display at native size if smaller than bounds.
    ScaleDown,
}

impl Default for ImageFitMode {
    fn default() -> Self {
        ImageFitMode::Contain
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputMode {
    Passthrough,
    #[default]
    Capture,
    LocalOnly,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontFamily {
    #[default]
    SystemSansSerif,
    SystemMonospace,
    SystemSerif,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

// ─── Scene Objects ──────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: SceneId,
    pub name: String,
    pub display_order: u32,
    pub created_at_ms: u64,
}

/// Per-agent resource envelope enforced by the budget enforcement ladder.
///
/// All four dimensions are checked at mutation intake (Stage 3). A mutation batch
/// that would push any dimension over budget is rejected whole (all-or-nothing).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceBudget {
    /// Maximum number of tiles this agent may hold simultaneously.
    pub max_tiles: u32,
    /// Maximum texture memory across all tiles, in bytes.
    pub max_texture_bytes: u64,
    /// Maximum scene mutation rate in Hz (sliding window over last 1 second).
    pub max_update_rate_hz: f32,
    /// Maximum nodes per individual tile.
    pub max_nodes_per_tile: u32,
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_tiles: 8,
            max_texture_bytes: 256 * 1024 * 1024, // 256 MiB
            max_update_rate_hz: 30.0,
            max_nodes_per_tile: 32,
        }
    }
}

/// A dimension in which an agent has violated its resource budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BudgetViolation {
    /// Agent holds more tiles than `max_tiles`.
    TileCountExceeded {
        current: u32,
        limit: u32,
    },
    /// Texture memory across all tiles exceeds `max_texture_bytes`.
    TextureMemoryExceeded {
        current_bytes: u64,
        limit_bytes: u64,
    },
    /// Scene mutation rate exceeds `max_update_rate_hz`.
    UpdateRateExceeded {
        current_hz: f32,
        limit_hz: f32,
    },
    /// A single tile contains more nodes than `max_nodes_per_tile`.
    NodeCountPerTileExceeded {
        tile_id_hint: String,
        current: u32,
        limit: u32,
    },
    /// Mutation would push texture memory past the absolute hard maximum.
    /// This is a critical violation — session is revoked immediately.
    CriticalTextureOomAttempt {
        requested_bytes: u64,
        hard_max_bytes: u64,
    },
    /// Session has accumulated too many protocol invariant violations.
    /// This is a critical violation — session is revoked immediately.
    RepeatedInvariantViolations {
        count: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub id: SceneId,
    pub tab_id: SceneId,
    pub namespace: String,
    pub lease_id: SceneId,
    pub bounds: Rect,
    pub z_order: u32,
    pub opacity: f32,
    pub input_mode: InputMode,
    pub sync_group: Option<SceneId>,
    pub present_at: Option<u64>,
    pub expires_at: Option<u64>,
    pub resource_budget: ResourceBudget,
    pub root_node: Option<SceneId>,
}

// ─── Nodes ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: SceneId,
    pub children: Vec<SceneId>,
    pub data: NodeData,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum NodeData {
    SolidColor(SolidColorNode),
    TextMarkdown(TextMarkdownNode),
    HitRegion(HitRegionNode),
    StaticImage(StaticImageNode),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolidColorNode {
    pub color: Rgba,
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextMarkdownNode {
    pub content: String,
    pub bounds: Rect,
    pub font_size_px: f32,
    pub font_family: FontFamily,
    pub color: Rgba,
    pub background: Option<Rgba>,
    pub alignment: TextAlign,
    pub overflow: TextOverflow,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitRegionNode {
    pub bounds: Rect,
    pub interaction_id: String,
    pub accepts_focus: bool,
    pub accepts_pointer: bool,
}

/// A static image node that displays raw RGBA pixel data within the node's bounds.
///
/// Image data is content-addressed via `content_hash` for deduplication by the compositor.
/// The `fit_mode` controls how the image is scaled to fill the bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaticImageNode {
    /// Raw RGBA8 pixel data (width * height * 4 bytes).
    pub image_data: Vec<u8>,
    /// Width of the image in pixels.
    pub width: u32,
    /// Height of the image in pixels.
    pub height: u32,
    /// SHA-256 hex string for content-based deduplication.
    pub content_hash: String,
    /// How the image is fitted within `bounds`.
    pub fit_mode: ImageFitMode,
    /// Position and size within the parent tile (in tile-local coordinates).
    pub bounds: Rect,
}

// ─── Hit Region Local State (compositor-managed) ────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitRegionLocalState {
    pub node_id: SceneId,
    pub hovered: bool,
    pub pressed: bool,
    pub focused: bool,
}

impl HitRegionLocalState {
    pub fn new(node_id: SceneId) -> Self {
        Self {
            node_id,
            hovered: false,
            pressed: false,
            focused: false,
        }
    }
}

// ─── Lease state machine (RFC 0008) ──────────────────────────────────────────

/// Lease lifecycle state per RFC 0008 SS3.
///
/// Terminal states: `Revoked`, `Expired`, `Released`.
/// Non-terminal: `Requested`, `Active`, `Suspended`, `Disconnected`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaseState {
    /// Lease requested but not yet granted.
    Requested,
    /// Lease is active — mutations allowed.
    Active,
    /// Lease suspended (safe mode, freeze) — mutations blocked, state preserved.
    Suspended,
    /// Agent disconnected — in grace period before cleanup.
    Disconnected,
    /// Lease revoked — state destroyed.
    Revoked,
    /// Lease expired (TTL exceeded) — state destroyed.
    Expired,
    /// Agent voluntarily released lease — state destroyed.
    Released,
}

impl LeaseState {
    /// Whether this state is terminal (no further transitions possible).
    pub fn is_terminal(self) -> bool {
        matches!(self, LeaseState::Revoked | LeaseState::Expired | LeaseState::Released)
    }
}

/// Renewal policy per RFC 0008 SS1.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenewalPolicy {
    /// Agent must explicitly renew before TTL expires.
    Manual,
    /// Runtime auto-renews at 75% TTL elapsed.
    AutoRenew,
    /// No renewal; expires at TTL.
    OneShot,
}

impl Default for RenewalPolicy {
    fn default() -> Self {
        RenewalPolicy::Manual
    }
}

/// Error type for lease state transitions.
#[derive(Clone, Debug, PartialEq)]
pub enum LeaseError {
    /// Attempted an invalid state transition.
    InvalidTransition { from: LeaseState, to: LeaseState },
    /// Lease not found in the scene graph.
    LeaseNotFound(SceneId),
    /// Lease exists but is not in Active state.
    LeaseNotActive(SceneId),
    /// Mutation would exceed the lease's resource budget.
    BudgetExceeded(BudgetError),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeaseError::InvalidTransition { from, to } => {
                write!(f, "invalid lease transition: {:?} -> {:?}", from, to)
            }
            LeaseError::LeaseNotFound(id) => write!(f, "lease not found: {}", id),
            LeaseError::LeaseNotActive(id) => write!(f, "lease not active: {}", id),
            LeaseError::BudgetExceeded(e) => write!(f, "budget exceeded: {}", e),
        }
    }
}

impl std::error::Error for LeaseError {}

/// Error returned when a mutation batch would exceed budget limits.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetError {
    /// Which resource dimension was exceeded (e.g. "tiles", "texture_bytes").
    pub resource: String,
    /// Current usage before the mutation.
    pub current: u64,
    /// The configured limit.
    pub limit: u64,
    /// How much the mutation batch would add.
    pub requested: u64,
}

impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: current={}, limit={}, requested={}",
            self.resource, self.current, self.limit, self.requested
        )
    }
}

/// Current resource usage for a lease.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceUsage {
    /// Number of tiles owned by this lease.
    pub tiles: u32,
    /// Total texture bytes across all tiles.
    pub texture_bytes: u64,
    /// Node count per tile (tile_id -> count).
    pub nodes_per_tile: HashMap<SceneId, u32>,
}

/// Information about an expired or cleaned-up lease.
#[derive(Clone, Debug, PartialEq)]
pub struct LeaseExpiry {
    /// The lease ID that was expired/cleaned up.
    pub lease_id: SceneId,
    /// The terminal state it entered.
    pub terminal_state: LeaseState,
    /// Tile IDs that were removed as a result.
    pub removed_tiles: Vec<SceneId>,
}

// ─── Lease ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lease {
    pub id: SceneId,
    pub namespace: String,
    pub state: LeaseState,
    /// Priority: 0=system/chrome, 1-3=agent, 4+=background (RFC 0008 SS2).
    pub priority: u32,
    pub granted_at_ms: u64,
    pub ttl_ms: u64,
    pub renewal_policy: RenewalPolicy,
    pub capabilities: Vec<Capability>,
    pub resource_budget: ResourceBudget,
    // Suspension tracking
    /// Timestamp when the lease was suspended (ms since epoch).
    pub suspended_at_ms: Option<u64>,
    /// TTL remaining at the moment of suspension (ms).
    pub ttl_remaining_at_suspend_ms: Option<u64>,
    // Disconnect tracking
    /// Timestamp when the agent disconnected (ms since epoch).
    pub disconnected_at_ms: Option<u64>,
    /// Grace period before a disconnected lease is cleaned up (ms). Default 30_000.
    pub grace_period_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    CreateTile,
    UpdateTile,
    DeleteTile,
    CreateNode,
    UpdateNode,
    DeleteNode,
    ReceiveInput,
}

impl Lease {
    /// Check if the lease has expired based on effective TTL elapsed.
    ///
    /// Accounts for suspension: time spent in Suspended state does not count
    /// toward TTL consumption (RFC 0008 SS4.3).
    pub fn is_expired(&self, now_ms: u64) -> bool {
        match self.state {
            // Terminal states are already past expiry semantics.
            LeaseState::Revoked | LeaseState::Expired | LeaseState::Released => true,
            // When suspended, TTL clock is paused — not expired.
            LeaseState::Suspended => false,
            // When disconnected, TTL continues from disconnected_at_ms.
            // (Grace period handles cleanup separately.)
            LeaseState::Disconnected | LeaseState::Active | LeaseState::Requested => {
                self.effective_remaining_ms(now_ms) == 0
            }
        }
    }

    pub fn has_capability(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Remaining TTL in milliseconds (0 if expired).
    ///
    /// If the lease was previously suspended, the suspension duration is
    /// deducted so that the effective TTL is preserved across suspend/resume.
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.effective_remaining_ms(now_ms)
    }

    /// Effective remaining TTL accounting for suspension pauses.
    fn effective_remaining_ms(&self, now_ms: u64) -> u64 {
        match self.state {
            LeaseState::Suspended => {
                // TTL frozen at the value saved when suspension started.
                self.ttl_remaining_at_suspend_ms.unwrap_or(0)
            }
            _ => {
                let expires = self.granted_at_ms + self.ttl_ms;
                expires.saturating_sub(now_ms)
            }
        }
    }

    // ─── State transition methods ────────────────────────────────────────

    /// Transition Active -> Suspended (safe mode entry).
    ///
    /// Pauses the TTL clock and records suspension timestamp.
    pub fn suspend(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Active {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Suspended,
            });
        }
        let remaining = self.effective_remaining_ms(now_ms);
        self.suspended_at_ms = Some(now_ms);
        self.ttl_remaining_at_suspend_ms = Some(remaining);
        self.state = LeaseState::Suspended;
        Ok(())
    }

    /// Transition Suspended -> Active (safe mode exit).
    ///
    /// Resumes the TTL clock. The `granted_at_ms` and `ttl_ms` are adjusted
    /// so that the remaining TTL equals what was saved at suspension time.
    pub fn resume(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Suspended {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        // Restore TTL: set granted_at_ms so that granted_at_ms + ttl_ms
        // equals now_ms + remaining.
        if let Some(remaining) = self.ttl_remaining_at_suspend_ms {
            self.granted_at_ms = now_ms;
            self.ttl_ms = remaining;
        }
        self.suspended_at_ms = None;
        self.ttl_remaining_at_suspend_ms = None;
        self.state = LeaseState::Active;
        Ok(())
    }

    /// Transition Active -> Disconnected (agent disconnect).
    ///
    /// Starts the grace period. TTL continues running.
    pub fn disconnect(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Active {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Disconnected,
            });
        }
        self.disconnected_at_ms = Some(now_ms);
        self.state = LeaseState::Disconnected;
        Ok(())
    }

    /// Transition Disconnected -> Active (agent reconnect within grace period).
    pub fn reconnect(&mut self, now_ms: u64) -> Result<(), LeaseError> {
        if self.state != LeaseState::Disconnected {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        // Check that grace period has not expired.
        if self.check_grace_expired(now_ms) {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Active,
            });
        }
        self.disconnected_at_ms = None;
        self.state = LeaseState::Active;
        Ok(())
    }

    /// Transition any non-terminal state -> Revoked.
    pub fn revoke(&mut self) -> Result<(), LeaseError> {
        if self.state.is_terminal() {
            return Err(LeaseError::InvalidTransition {
                from: self.state,
                to: LeaseState::Revoked,
            });
        }
        self.state = LeaseState::Revoked;
        Ok(())
    }

    /// Whether the lease is currently in Active state.
    pub fn is_active(&self) -> bool {
        self.state == LeaseState::Active
    }

    /// Whether mutations are allowed. Only Active state permits mutations.
    pub fn is_mutations_allowed(&self) -> bool {
        self.state == LeaseState::Active
    }

    /// Check if the grace period has expired for a disconnected lease.
    pub fn check_grace_expired(&self, now_ms: u64) -> bool {
        match (self.state, self.disconnected_at_ms) {
            (LeaseState::Disconnected, Some(disc_at)) => {
                now_ms >= disc_at + self.grace_period_ms
            }
            _ => false,
        }
    }

    /// Check if a suspended lease has exceeded the maximum suspension time.
    pub fn check_suspension_expired(&self, now_ms: u64, max_suspend_ms: u64) -> bool {
        match (self.state, self.suspended_at_ms) {
            (LeaseState::Suspended, Some(susp_at)) => {
                now_ms >= susp_at + max_suspend_ms
            }
            _ => false,
        }
    }
}

// ─── Sync Groups ────────────────────────────────────────────────────────────

/// Type alias for sync group IDs (they are just SceneIds).
pub type SyncGroupId = SceneId;

/// Commit policy for a sync group.
///
/// See RFC 0003 §2.2 for full semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncCommitPolicy {
    /// All members must have a pending mutation before any are applied.
    /// If not all members are ready when Stage 4 runs, the group is deferred.
    /// After `max_deferrals` consecutive deferrals the available members are
    /// force-committed and a telemetry event is emitted.
    #[default]
    AllOrDefer,

    /// Apply whatever subset of members have pending mutations this frame.
    /// Members without pending mutations are implicitly "unchanged".
    AvailableMembers,
}

/// A sync group is a named set of tiles whose mutations must be applied
/// atomically in the same frame.
///
/// See RFC 0003 §2 for the full specification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncGroup {
    /// Unique identifier (UUIDv7).
    pub id: SyncGroupId,
    /// Optional human-readable label (max 128 UTF-8 bytes).
    pub name: Option<String>,
    /// Namespace that created this group.
    pub owner_namespace: String,
    /// Tile IDs currently in the group.
    pub members: std::collections::BTreeSet<SceneId>,
    /// Wall-clock creation time (UTC microseconds since Unix epoch).
    pub created_at_us: u64,
    /// Commit policy.
    pub commit_policy: SyncCommitPolicy,
    /// Maximum number of consecutive deferral frames before a force-commit.
    /// Only relevant when `commit_policy == AllOrDefer`. Default: 3.
    pub max_deferrals: u32,
    /// Current consecutive deferral count (runtime state — not part of
    /// the authoritative scene snapshot, but carried in the struct for
    /// simplicity in the scene crate).
    #[serde(default)]
    pub deferral_count: u32,
}

impl SyncGroup {
    pub fn new(
        id: SyncGroupId,
        name: Option<String>,
        owner_namespace: String,
        commit_policy: SyncCommitPolicy,
        max_deferrals: u32,
        created_at_us: u64,
    ) -> Self {
        Self {
            id,
            name,
            owner_namespace,
            members: std::collections::BTreeSet::new(),
            created_at_us,
            commit_policy,
            max_deferrals,
            deferral_count: 0,
        }
    }
}

// ─── Zone types ─────────────────────────────────────────────────────────────

/// Display edge for edge-anchored zone geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayEdge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Geometry policy — how a zone is positioned on the display.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum GeometryPolicy {
    /// Percentage-based position relative to display area.
    Relative {
        x_pct: f32,
        y_pct: f32,
        width_pct: f32,
        height_pct: f32,
    },
    /// Anchored to a display edge.
    EdgeAnchored {
        edge: DisplayEdge,
        /// Used for Top/Bottom edges.
        height_pct: f32,
        /// Used for Left/Right edges.
        width_pct: f32,
        margin_px: f32,
    },
}

/// Media types that can be published to a zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneMediaType {
    /// Stream-text with optional breakpoints.
    StreamText,
    /// Notification: text + icon + urgency.
    ShortTextWithIcon,
    /// Status-bar: key-value map.
    KeyValuePairs,
    /// Reference to a media surface (post-v1 media layer).
    VideoSurfaceRef,
    /// Static image resource.
    StaticImage,
    /// Solid color fill.
    SolidColor,
}

/// Rendering policy — how content is presented in the zone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RenderingPolicy {
    pub font_size_px: Option<f32>,
    pub backdrop: Option<Rgba>,
    pub text_align: Option<TextAlign>,
    pub margin_px: Option<f32>,
}

/// Contention policy — what happens when multiple agents publish to the same zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentionPolicy {
    /// Most recent publish replaces previous content.
    LatestWins,
    /// Publishes accumulate as a stack; each auto-dismisses.
    Stack { max_depth: u8 },
    /// Each publish includes a key; same key replaces, different keys coexist.
    MergeByKey { max_keys: u8 },
    /// Only one occupant; new publish evicts current one.
    Replace,
}

/// Transport constraint for zone publishing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportConstraint {
    /// Content must arrive via gRPC session stream.
    GrpcOnly,
    /// Content may arrive via MCP tool call.
    McpAllowed,
    /// Content requires WebRTC media channel (post-v1).
    WebRtcRequired,
}

/// Full zone definition per RFC 0001 §2.5.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneDefinition {
    pub id: SceneId,
    pub name: String,
    pub description: String,
    pub geometry_policy: GeometryPolicy,
    pub accepted_media_types: Vec<ZoneMediaType>,
    pub rendering_policy: RenderingPolicy,
    pub contention_policy: ContentionPolicy,
    pub max_publishers: u32,
    pub transport_constraint: Option<TransportConstraint>,
    /// Auto-clear timeout in milliseconds; None = no auto-clear.
    pub auto_clear_ms: Option<u64>,
}

// ─── Zone publish token ──────────────────────────────────────────────────────

/// Opaque capability token that authorizes publishing to a specific zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZonePublishToken {
    /// Opaque bytes issued at session auth.
    pub token: Vec<u8>,
}

// ─── Zone content ────────────────────────────────────────────────────────────

/// Notification payload: text + optional icon + urgency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPayload {
    pub text: String,
    /// Resource name or empty string.
    pub icon: String,
    /// 0=low, 1=normal, 2=urgent, 3=critical.
    pub urgency: u32,
}

/// Status-bar payload: key → display string map.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusBarPayload {
    pub entries: HashMap<String, String>,
}

/// Content that can be published to a zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ZoneContent {
    StreamText(String),
    Notification(NotificationPayload),
    StatusBar(StatusBarPayload),
    SolidColor(Rgba),
}

// ─── Zone publish records ────────────────────────────────────────────────────

/// Record of a single publish event into a zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZonePublishRecord {
    pub zone_name: String,
    pub publisher_namespace: String,
    pub content: ZoneContent,
    pub published_at_ms: u64,
    /// For MergeByKey contention: the key under which this record is stored.
    pub merge_key: Option<String>,
}

// ─── Zone registry ───────────────────────────────────────────────────────────

/// Snapshot of the zone registry (all zones + active publishes).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneRegistrySnapshot {
    pub zones: Vec<ZoneDefinition>,
    pub active_publishes: Vec<ZonePublishRecord>,
}

/// Runtime-owned zone registry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneRegistry {
    /// Zone definitions keyed by zone name.
    pub zones: HashMap<String, ZoneDefinition>,
    /// Active publishes per zone name.
    /// For LatestWins/Replace: at most one entry per zone.
    /// For Stack: ordered oldest-first, bounded by max_depth.
    /// For MergeByKey: keyed by merge_key, bounded by max_keys.
    pub active_publishes: HashMap<String, Vec<ZonePublishRecord>>,
}

impl ZoneRegistry {
    pub fn new() -> Self {
        Self {
            zones: HashMap::new(),
            active_publishes: HashMap::new(),
        }
    }

    /// Create a registry pre-populated with the default v1 zones.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        // status-bar zone: edge-anchored bottom, MergeByKey
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "status-bar".to_string(),
            description: "Status bar at the bottom of the display".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.04,
                width_pct: 1.0,
                margin_px: 0.0,
            },
            accepted_media_types: vec![ZoneMediaType::KeyValuePairs],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::MergeByKey { max_keys: 32 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: None,
        });

        // notification-area zone: edge-anchored top-right, Stack
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "notification-area".to_string(),
            description: "Notification overlay area".to_string(),
            geometry_policy: GeometryPolicy::Relative {
                x_pct: 0.75,
                y_pct: 0.02,
                width_pct: 0.24,
                height_pct: 0.30,
            },
            accepted_media_types: vec![ZoneMediaType::ShortTextWithIcon],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::Stack { max_depth: 8 },
            max_publishers: 16,
            transport_constraint: None,
            auto_clear_ms: Some(5_000),
        });

        // subtitle zone: edge-anchored bottom, LatestWins
        registry.register(ZoneDefinition {
            id: SceneId::new(),
            name: "subtitle".to_string(),
            description: "Subtitle / caption overlay".to_string(),
            geometry_policy: GeometryPolicy::EdgeAnchored {
                edge: DisplayEdge::Bottom,
                height_pct: 0.10,
                width_pct: 0.80,
                margin_px: 48.0,
            },
            accepted_media_types: vec![ZoneMediaType::StreamText],
            rendering_policy: RenderingPolicy::default(),
            contention_policy: ContentionPolicy::LatestWins,
            max_publishers: 1,
            transport_constraint: None,
            auto_clear_ms: None,
        });

        registry
    }

    /// Register a zone definition. Overwrites any existing definition with the same name.
    pub fn register(&mut self, zone: ZoneDefinition) {
        self.zones.insert(zone.name.clone(), zone);
    }

    /// Remove a zone definition by name. Returns the removed definition if present.
    pub fn unregister(&mut self, name: &str) -> Option<ZoneDefinition> {
        self.active_publishes.remove(name);
        self.zones.remove(name)
    }

    /// Look up a zone by name.
    pub fn get_by_name(&self, name: &str) -> Option<&ZoneDefinition> {
        self.zones.get(name)
    }

    /// Query zones that accept a given media type.
    pub fn zones_accepting(&self, media_type: ZoneMediaType) -> Vec<&ZoneDefinition> {
        self.zones
            .values()
            .filter(|z| z.accepted_media_types.contains(&media_type))
            .collect()
    }

    /// Return all zone definitions.
    pub fn all_zones(&self) -> Vec<&ZoneDefinition> {
        self.zones.values().collect()
    }

    /// Get the current active publish(es) for a zone.
    pub fn active_for_zone(&self, zone_name: &str) -> &[ZonePublishRecord] {
        self.active_publishes
            .get(zone_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Snapshot the registry (all definitions + all active publishes).
    pub fn snapshot(&self) -> ZoneRegistrySnapshot {
        ZoneRegistrySnapshot {
            zones: self.zones.values().cloned().collect(),
            active_publishes: self
                .active_publishes
                .values()
                .flat_map(|v| v.iter().cloned())
                .collect(),
        }
    }
}

impl Default for ZoneRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    // ── SceneId size invariant ────────────────────────────────────────────────

    #[test]
    fn scene_id_size_is_16_bytes() {
        assert_eq!(size_of::<SceneId>(), 16, "SceneId must be exactly 16 bytes");
    }

    // ── SceneId null sentinel ─────────────────────────────────────────────────

    #[test]
    fn scene_id_null_is_all_zeros() {
        let null = SceneId::null();
        assert!(null.is_null(), "SceneId::null() must report is_null() == true");
        assert_eq!(null.to_bytes_le(), [0u8; 16], "null SceneId must serialize to 16 zero bytes");
    }

    #[test]
    fn scene_id_new_is_never_null() {
        let id = SceneId::new();
        assert!(!id.is_null(), "freshly-generated SceneId must not be null");
    }

    #[test]
    fn scene_id_nil_aliases_null() {
        assert_eq!(SceneId::nil(), SceneId::null());
        assert!(SceneId::nil().is_nil());
    }

    // ── SceneId byte round-trip ───────────────────────────────────────────────

    #[test]
    fn scene_id_bytes_le_round_trip() {
        let id = SceneId::new();
        let bytes = id.to_bytes_le();
        let restored = SceneId::from_bytes_le(&bytes).expect("must decode 16 bytes");
        assert_eq!(id, restored, "SceneId bytes LE round-trip must be lossless");
    }

    #[test]
    fn scene_id_from_bytes_le_rejects_wrong_length() {
        assert!(SceneId::from_bytes_le(&[0u8; 15]).is_none());
        assert!(SceneId::from_bytes_le(&[0u8; 17]).is_none());
        assert!(SceneId::from_bytes_le(&[]).is_none());
    }

    #[test]
    fn scene_id_null_round_trips_via_bytes() {
        let null = SceneId::null();
        let bytes = null.to_bytes_le();
        let restored = SceneId::from_bytes_le(&bytes).unwrap();
        assert!(restored.is_null());
    }

    // ── SceneId lexicographic / monotonicity ─────────────────────────────────

    #[test]
    fn scene_id_monotonic_small_batch() {
        // Generate a small batch synchronously and verify they're non-decreasing.
        // (A full 10,000-ID property test is in the proptest suite below.)
        let ids: Vec<SceneId> = (0..64).map(|_| SceneId::new()).collect();
        for w in ids.windows(2) {
            assert!(
                w[0] <= w[1],
                "SceneId sequence must be non-decreasing: {:?} > {:?}",
                w[0],
                w[1]
            );
        }
    }

    // ── ResourceId size invariant ─────────────────────────────────────────────

    #[test]
    fn resource_id_size_is_32_bytes() {
        assert_eq!(size_of::<ResourceId>(), 32, "ResourceId must be exactly 32 bytes");
    }

    // ── ResourceId content deduplication ─────────────────────────────────────

    #[test]
    fn resource_id_same_content_same_id() {
        let data = b"hello world";
        let id1 = ResourceId::of(data);
        let id2 = ResourceId::of(data);
        assert_eq!(id1, id2, "identical content must produce the same ResourceId");
    }

    #[test]
    fn resource_id_different_content_different_id() {
        let id1 = ResourceId::of(b"foo");
        let id2 = ResourceId::of(b"bar");
        assert_ne!(id1, id2, "different content must produce different ResourceIds");
    }

    #[test]
    fn resource_id_empty_content() {
        let id = ResourceId::of(b"");
        assert_eq!(id.as_bytes().len(), 32);
    }

    // ── ResourceId byte round-trip ────────────────────────────────────────────

    #[test]
    fn resource_id_from_bytes_round_trip() {
        let id = ResourceId::of(b"round-trip test payload");
        let bytes = *id.as_bytes();
        let restored = ResourceId::from_bytes(bytes);
        assert_eq!(id, restored);
    }

    #[test]
    fn resource_id_from_slice_round_trip() {
        let id = ResourceId::of(b"slice round-trip");
        let restored = ResourceId::from_slice(id.as_bytes()).expect("must accept 32-byte slice");
        assert_eq!(id, restored);
    }

    #[test]
    fn resource_id_from_slice_rejects_wrong_length() {
        assert!(ResourceId::from_slice(&[0u8; 31]).is_none());
        assert!(ResourceId::from_slice(&[0u8; 33]).is_none());
        assert!(ResourceId::from_slice(&[]).is_none());
    }

    // ── ResourceId display / hex is debug-only ────────────────────────────────

    #[test]
    fn resource_id_to_hex_is_64_chars() {
        let id = ResourceId::of(b"hex display test");
        let hex = id.to_hex();
        assert_eq!(hex.len(), 64, "hex of 32-byte hash must be 64 chars");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()), "must be valid hex");
    }

    // ── Layer 0 identity invariant check helper ───────────────────────────────

    /// Validates the core Layer 0 identity invariants for `SceneId` and `ResourceId`.
    /// This function mirrors what `assert_layer0_invariants` checks at the graph level
    /// but focuses on the type-level contracts.
    pub fn assert_identity_invariants() -> Vec<String> {
        let mut violations = Vec::new();

        if size_of::<SceneId>() != 16 {
            violations.push(format!("SceneId size {} != 16", size_of::<SceneId>()));
        }
        if size_of::<ResourceId>() != 32 {
            violations.push(format!("ResourceId size {} != 32", size_of::<ResourceId>()));
        }
        if !SceneId::null().is_null() {
            violations.push("SceneId::null() does not report is_null()".into());
        }
        if SceneId::new().is_null() {
            violations.push("freshly-generated SceneId reports is_null()".into());
        }
        let id = ResourceId::of(b"test");
        if ResourceId::of(b"test") != id {
            violations.push("ResourceId deduplication failed".into());
        }

        violations
    }

    #[test]
    fn layer0_identity_invariants_pass() {
        let violations = assert_identity_invariants();
        assert!(violations.is_empty(), "Layer 0 identity violations: {:?}", violations);
    }
}

// ─── Property tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Generates 10,000 SceneIds and asserts they are monotonically non-decreasing.
        ///
        /// UUIDv7 guarantees creation-time ordering, so lexicographic sort == chronological sort.
        #[test]
        fn scene_id_monotonic_10k(_ in 0u32..1u32) {
            let ids: Vec<SceneId> = (0..10_000).map(|_| SceneId::new()).collect();
            for w in ids.windows(2) {
                prop_assert!(
                    w[0] <= w[1],
                    "SceneId not monotonically non-decreasing: {:?} > {:?}",
                    w[0],
                    w[1]
                );
            }
        }

        /// Verifies that any 16-byte input round-trips through SceneId bytes LE encoding.
        #[test]
        fn scene_id_bytes_le_roundtrip_arb(raw in proptest::array::uniform16(0u8..)) {
            // from_bytes_le -> to_bytes_le must be identity
            let id = SceneId::from_bytes_le(&raw).expect("uniform16 is always 16 bytes");
            prop_assert_eq!(id.to_bytes_le(), raw);
        }

        /// Verifies that any 32-byte slice round-trips through ResourceId.
        #[test]
        fn resource_id_bytes_roundtrip_arb(raw in proptest::array::uniform32(0u8..)) {
            let id = ResourceId::from_bytes(raw);
            prop_assert_eq!(*id.as_bytes(), raw);
        }

        /// Verifies BLAKE3 determinism: same input always produces the same ResourceId.
        #[test]
        fn resource_id_deterministic(data in proptest::collection::vec(0u8.., 0..1024)) {
            let id1 = ResourceId::of(&data);
            let id2 = ResourceId::of(&data);
            prop_assert_eq!(id1, id2);
        }

        /// Verifies that distinct inputs produce distinct ResourceIds (collision resistance).
        #[test]
        fn resource_id_distinct_inputs_distinct_ids(
            a in proptest::collection::vec(0u8.., 1..512),
            b in proptest::collection::vec(0u8.., 1..512),
        ) {
            // Only assert when inputs differ
            if a != b {
                let id_a = ResourceId::of(&a);
                let id_b = ResourceId::of(&b);
                prop_assert_ne!(id_a, id_b, "distinct content must yield distinct ResourceIds");
            }
        }
    }
}
