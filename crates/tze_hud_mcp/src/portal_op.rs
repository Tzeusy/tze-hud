//! Portal projection operation channel messages.
//!
//! This module defines the [`PortalOp`] enum that flows from the MCP HTTP
//! server task (async Tokio thread) to the winit event-loop thread via an
//! unbounded mpsc channel.
//!
//! ## Why is this in `tze_hud_mcp`?
//!
//! The MCP server needs to _send_ `PortalOp` values; the runtime
//! (`tze_hud_runtime`) needs to _receive_ and _dispatch_ them. Since
//! `tze_hud_runtime` depends on `tze_hud_mcp` (not the other way around),
//! placing the type here avoids a circular dependency while keeping the
//! channel definition close to the MCP tool implementations that produce it.
//!
//! The runtime re-exports `PortalOp` from
//! `tze_hud_runtime::portal_projection_driver`.
//!
//! ## Field types
//!
//! All fields use plain [`String`] / [`Option<String>`] so this module has no
//! dependency on `tze_hud_projection`. The runtime converts them to the
//! appropriate projection types (`ProviderKind`, `OutputKind`, etc.) before
//! calling the authority.

/// An operation dispatched to the in-process projection authority through the
/// `portal_op_tx` / `portal_op_rx` channel pair (hud-bq0gl.2).
///
/// The MCP layer sends one of these for each `portal_projection_attach` or
/// `portal_projection_publish` tool call. The winit event-loop thread drains
/// the channel on every `about_to_wait` iteration (via `drain_portal_ops`) and
/// applies the operation to the in-process `InProcessPortalDriver` before
/// running the normal drain loop.
///
/// ## Why a channel?
///
/// The `InProcessPortalDriver` lives on the winit event-loop thread and is
/// accessed synchronously from `about_to_wait`. The MCP HTTP server runs on a
/// Tokio async runtime thread. An unbounded mpsc channel is the correct bridge:
/// the MCP task sends and returns immediately; the event-loop thread drains
/// lock-free on each iteration without blocking.
///
/// ## Security
///
/// `owner_token` for `PublishOutput` is validated by the projection authority
/// (`handle_publish_output`). The MCP layer forwards it verbatim; the authority
/// rejects invalid or expired tokens before storing any content.
#[derive(Debug)]
pub enum PortalOp {
    /// Attach a new projection session to the in-process authority.
    Attach {
        /// Caller-assigned identifier (max 128 bytes, must be unique).
        projection_id: String,
        /// Human-readable name for the session.
        display_name: String,
        /// Optional idempotency key for replay-safe re-attach.
        idempotency_key: Option<String>,
        /// One-shot response channel: authority returns the `owner_token` on
        /// success (needed for subsequent `PublishOutput` calls), or an error
        /// description on failure.
        reply: tokio::sync::oneshot::Sender<Result<String, String>>,
    },
    /// Publish output text to an existing projection session.
    PublishOutput {
        /// Projection identifier matching a prior successful `Attach`.
        projection_id: String,
        /// Owner token returned by the `Attach` response.
        owner_token: String,
        /// Text to append to the transcript.
        output_text: String,
        /// Optional logical-unit ID for idempotent replay detection.
        logical_unit_id: Option<String>,
        /// Optional output kind as a snake_case string (`assistant`, `tool`,
        /// `status`, `error`, `other`). The runtime parses this into
        /// `OutputKind`, defaulting to `assistant` when `None`. An
        /// unrecognized value is rejected by the runtime.
        output_kind: Option<String>,
        /// Optional viewer-facing content classification as a snake_case
        /// string (`public`, `household`, `private`, `sensitive`). The
        /// runtime parses this into `ContentClassification`, defaulting to
        /// the safe-by-default `private` when `None`. An unrecognized value
        /// is rejected by the runtime.
        content_classification: Option<String>,
        /// Optional coalesce key. When set, repeated publishes sharing the
        /// key collapse in-place into a single transcript unit rather than
        /// appending. `None` means append (no coalescing).
        coalesce_key: Option<String>,
        /// One-shot response channel: `Ok(())` on success or an error
        /// description on validation / auth failure.
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}
