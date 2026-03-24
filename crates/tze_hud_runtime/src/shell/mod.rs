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
//! See `chrome.rs` for chrome layer implementation.
//! See `freeze.rs` for freeze semantics.

pub mod chrome;
pub mod freeze;

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
// ChromeDrawCmd lives in tze_hud_compositor to avoid circular dependencies.
pub use tze_hud_compositor::ChromeDrawCmd;

pub use freeze::{
    classify_mutation_batch, EnqueueResult, FreezeManager, FreezeQueue, FreezeState,
    MutationTrafficClass, QueuedMutation, DEFAULT_AUTO_UNFREEZE_MS,
    DEFAULT_FREEZE_QUEUE_CAPACITY, QUEUE_PRESSURE_FRACTION,
};
