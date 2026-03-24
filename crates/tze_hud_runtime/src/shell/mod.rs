//! System shell subsystem.
//!
//! The system shell owns the chrome layer — the set of UI elements that are ALWAYS
//! rendered on top of all agent content and are NEVER accessible to agents.
//!
//! The shell layer also implements human-override semantics: chrome sovereignty,
//! safe mode, freeze, privacy redaction, and disconnection badges.
//!
//! # Sovereignty contract
//!
//! - Chrome renders above all agent tiles in every frame (background → content → chrome).
//! - No agent API exposes any chrome element or viewer context.
//! - Shell state transitions are the SOLE owner of chrome layer governance.
//! - The shell is the **sole** owner of `OverrideState` transitions (freeze, safe mode).
//!   No other subsystem may write these fields.
//! - Override controls are local, frame-bounded, unconditional, and cannot be vetoed.
//!
//! # Redaction (Shell #4)
//!
//! Privacy redaction is handled by [`redaction`]. The shell is the sole owner of
//! redaction rendering decisions. Agents are never notified. See `redaction.rs`.
//!
//! # Badges (Shell #5)
//!
//! Disconnection badges and backpressure signals are handled by [`badges`]. The
//! shell is the sole owner of badge rendering. Badge state is owned by the
//! `badges` subsystem (per-tile [`badges::TileBadgeState`] and frame-level
//! [`badges::BadgeFrame`]) and consumed by the chrome render pass.
//! Agents are never notified of badge state. See `badges.rs`.
//!
//! See `chrome.rs` for chrome layer implementation.
//! See `freeze.rs` for freeze semantics.

pub mod badges;
pub mod chrome;
pub mod freeze;
pub mod redaction;
pub mod safe_mode;

pub use chrome::{
    // Core state
    ChromeState,
    ChromeTab,
    TabBarPosition,
    ViewerClass,
    ViewerClassTransition,
    SystemHealth,
    // Layout
    ChromeLayout,
    // Rendering
    ChromeRenderer,
    // Keyboard
    ChromeShortcut,
    ShortcutResult,
    handle_shortcut,
    // Dismiss / override
    DismissTileResult,
    RevokeReason,
    // Audit
    ShellAuditEvent,
    AuditTrigger,
    AuditPayload,
    SafeModeEntryReason,
    ShellAuditSink,
    NoopAuditSink,
    CollectingAuditSink,
    // Agent exclusion
    AgentVisibleTopology,
    strip_chrome_from_topology,
    // Diagnostics
    DiagnosticSnapshot,
    collect_diagnostic,
};
pub use safe_mode::{
    // Core state
    ShellOverrideState,
    // Results
    LeaseResumeInfo,
    SafeModeEntryResult,
    SafeModeExitResult,
    // Input handling
    SafeModeInput,
    SafeModeInputResult,
    classify_safe_mode_input,
    // Controller
    SafeModeController,
};
// ChromeDrawCmd lives in tze_hud_compositor to avoid circular dependencies.
pub use tze_hud_compositor::ChromeDrawCmd;

pub use freeze::{
    classify_mutation_batch, EnqueueResult, FreezeManager, FreezeQueue, FreezeState,
    MutationTrafficClass, QueuedMutation, DEFAULT_AUTO_UNFREEZE_MS,
    DEFAULT_FREEZE_QUEUE_CAPACITY, QUEUE_PRESSURE_FRACTION,
};

pub use redaction::{
    // Content classification (ViewerClass is already re-exported from chrome above)
    ContentClassification,
    // Redaction style
    RedactionStyle,
    // Core evaluation
    is_tile_redacted,
    hit_regions_enabled,
    // Per-tile state
    TileRedactionState,
    // Placeholder rendering
    build_redaction_cmds,
    // Frame-level evaluation
    RedactionFrame,
    // Rendering constants
    PATTERN_CELL_PX,
    MAX_PATTERN_ACCENT_RECTS,
    REDACTION_BLANK_COLOR,
    REDACTION_PATTERN_BASE,
    REDACTION_PATTERN_ACCENT,
};

pub use badges::{
    // Per-tile badge state (written by control plane, read by chrome render pass)
    TileBadgeState,
    // Draw command builder
    build_badge_cmds,
    // Frame-level badge snapshot
    BadgeFrame,
    // Backpressure signals
    BackpressureSignal,
    // Rendering constants
    DISCONNECTED_CONTENT_OPACITY,
    DISCONNECTED_BADGE_OPACITY,
    BUDGET_WARNING_BORDER_PX,
    BUDGET_WARNING_BORDER_OPACITY,
    BUDGET_WARNING_AMBER_COLOR,
    DISCONNECTION_BADGE_SIZE_PX,
    DISCONNECTION_BADGE_OFFSET_PX,
    DISCONNECTION_BADGE_BG_COLOR,
    DISCONNECTION_BADGE_ICON_COLOR,
    DISCONNECTION_CONTENT_SCRIM_COLOR,
};
