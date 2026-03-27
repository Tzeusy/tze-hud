//! # Agent Event Rate Limiter
//!
//! Re-exports [`AgentEventRateLimiter`] and its associated constants from
//! `tze_hud_scene::events::emission`, which is the canonical home shared
//! between the protocol layer and the runtime layer.
//!
//! See `tze_hud_scene::events::emission` for the full implementation and
//! spec references.

pub use tze_hud_scene::events::emission::{AgentEventRateLimiter, DEFAULT_MAX_EVENTS_PER_SECOND};
