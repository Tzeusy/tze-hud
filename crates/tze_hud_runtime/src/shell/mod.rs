//! System shell subsystem.
//!
//! The system shell owns the chrome layer — the set of UI elements that are ALWAYS
//! rendered on top of all agent content and are NEVER accessible to agents.
//!
//! # Sovereignty contract
//!
//! - Chrome renders above all agent tiles in every frame (background → content → chrome).
//! - No agent API exposes any chrome element or viewer context.
//! - Shell state transitions are the SOLE owner of chrome layer governance.
//! - Override controls are local, frame-bounded, unconditional, and cannot be vetoed.
//!
//! See `chrome.rs` for full implementation.

pub mod chrome;

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
