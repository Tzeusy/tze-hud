//! # Agent Event Emission Handler
//!
//! Implements the agent event emission protocol per
//! scene-events/spec.md §5.1–§5.4, lines 107-133:
//!
//! - Capability-gated emission (`emit_scene_event:<bare_name>`)
//! - Bare name regex validation (`[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+`)
//! - Reserved prefix rejection (`system.` and `scene.`)
//! - 4 KB payload size limit
//! - Sliding-window rate limiting (default 10 events/second)
//! - Namespace prefixing: `agent.<namespace>.<bare_name>`
//!
//! ## Usage
//!
//! 1. Create one [`AgentEventHandler`] per agent session, supplying the agent's
//!    namespace and granted capabilities.
//! 2. Call [`AgentEventHandler::handle`] for each `EmitSceneEvent` request.
//! 3. The returned [`EmissionResult`] indicates whether the event was accepted
//!    and provides the fully-prefixed event type for delivery.

pub mod rate_limiter;

use std::time::Instant;

use tze_hud_scene::events::naming::{validate_bare_name, build_agent_event_type};

pub use rate_limiter::{AgentEventRateLimiter, DEFAULT_MAX_EVENTS_PER_SECOND};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum allowed payload size in bytes (spec line 122: "limited to 4KB maximum").
pub const MAX_PAYLOAD_BYTES: usize = 4096;

/// Capability prefix for agent event emission (spec line 108).
const EMIT_CAPABILITY_PREFIX: &str = "emit_scene_event:";

// ─── Error types ─────────────────────────────────────────────────────────────

/// Reasons an agent event emission was rejected.
///
/// Each variant maps to a wire error code string:
///
/// | Variant | Wire `error_code` |
/// |---------|-------------------|
/// | `CapabilityMissing` | `"AGENT_EVENT_CAPABILITY_MISSING"` |
/// | `RateLimitExceeded` | `"AGENT_EVENT_RATE_EXCEEDED"` |
/// | `PayloadTooLarge`   | `"AGENT_EVENT_PAYLOAD_TOO_LARGE"` |
/// | `InvalidName`       | `"AGENT_EVENT_INVALID_NAME"` |
/// | `ReservedPrefix`    | `"AGENT_EVENT_RESERVED_PREFIX"` |
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmissionError {
    /// The agent does not hold the `emit_scene_event:<bare_name>` capability
    /// (spec line 113).
    CapabilityMissing { required: String },
    /// The agent has exceeded the per-session sliding-window rate limit
    /// (spec line 133).
    RateLimitExceeded,
    /// The payload exceeds the 4 KB maximum (spec line 122).
    PayloadTooLarge { actual: usize, limit: usize },
    /// The bare name does not match `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+`.
    InvalidName { detail: String },
    /// The bare name starts with a reserved prefix (`system.` or `scene.`).
    ReservedPrefix { prefix: String },
}

impl EmissionError {
    /// Returns the canonical wire error code string for this variant.
    pub fn error_code(&self) -> &'static str {
        match self {
            EmissionError::CapabilityMissing { .. } => "AGENT_EVENT_CAPABILITY_MISSING",
            EmissionError::RateLimitExceeded => "AGENT_EVENT_RATE_EXCEEDED",
            EmissionError::PayloadTooLarge { .. } => "AGENT_EVENT_PAYLOAD_TOO_LARGE",
            EmissionError::InvalidName { .. } => "AGENT_EVENT_INVALID_NAME",
            EmissionError::ReservedPrefix { .. } => "AGENT_EVENT_RESERVED_PREFIX",
        }
    }

    /// Returns a human-readable error message.
    pub fn message(&self) -> String {
        match self {
            EmissionError::CapabilityMissing { required } => {
                format!("missing capability: {required}")
            }
            EmissionError::RateLimitExceeded => {
                format!(
                    "agent event rate limit exceeded ({}/s sliding window)",
                    DEFAULT_MAX_EVENTS_PER_SECOND
                )
            }
            EmissionError::PayloadTooLarge { actual, limit } => {
                format!("payload {actual} bytes exceeds {limit}-byte limit")
            }
            EmissionError::InvalidName { detail } => {
                format!("invalid bare name: {detail}")
            }
            EmissionError::ReservedPrefix { prefix } => {
                format!("bare name must not start with reserved prefix {prefix:?}")
            }
        }
    }
}

impl std::fmt::Display for EmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error_code(), self.message())
    }
}

impl std::error::Error for EmissionError {}

// ─── Emission result ─────────────────────────────────────────────────────────

/// Outcome of an [`AgentEventHandler::handle`] call.
///
/// On success, carries the fully-qualified event type ready for delivery.
/// On failure, carries a structured [`EmissionError`].
pub type EmissionResult = Result<EmissionOutcome, EmissionError>;

/// The accepted emission outcome — caller should dispatch this event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmissionOutcome {
    /// Fully-prefixed event type as delivered to subscribers.
    /// Example: `"agent.doorbell_agent.doorbell.ring"`
    pub delivered_event_type: String,
    /// The validated bare name (unchanged from the request).
    pub bare_name: String,
}

// ─── Handler ─────────────────────────────────────────────────────────────────

/// Per-session agent event emission handler.
///
/// Create one instance per agent session. All state (rate limiter) is
/// per-session, so concurrent sessions must each hold their own handler.
///
/// # Example
///
/// ```
/// use tze_hud_runtime::agent_events::AgentEventHandler;
///
/// let caps = vec!["emit_scene_event:doorbell.ring".to_string()];
/// let mut handler = AgentEventHandler::new("doorbell_agent", caps);
///
/// let payload = b"{}".to_vec();
/// let result = handler.handle("doorbell.ring", payload, std::time::Instant::now());
/// assert!(result.is_ok());
/// assert_eq!(result.unwrap().delivered_event_type, "agent.doorbell_agent.doorbell.ring");
/// ```
pub struct AgentEventHandler {
    /// Agent namespace (e.g. `"doorbell_agent"`).
    namespace: String,
    /// Granted capabilities for this session.
    capabilities: Vec<String>,
    /// Sliding-window rate limiter.
    rate_limiter: AgentEventRateLimiter,
}

impl AgentEventHandler {
    /// Create a new handler for the given agent namespace and capabilities.
    pub fn new(namespace: impl Into<String>, capabilities: Vec<String>) -> Self {
        Self {
            namespace: namespace.into(),
            capabilities,
            rate_limiter: AgentEventRateLimiter::new(),
        }
    }

    /// Create a handler with a custom rate limit (for testing or per-config overrides).
    pub fn with_rate_limit(
        namespace: impl Into<String>,
        capabilities: Vec<String>,
        max_per_second: u32,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            capabilities,
            rate_limiter: AgentEventRateLimiter::with_limit(max_per_second),
        }
    }

    /// Process an `EmitSceneEvent` request from the agent.
    ///
    /// Validation order:
    ///
    /// 1. Bare name format (`[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+`)
    /// 2. Reserved prefix check (`system.` / `scene.`)
    /// 3. Capability check (`emit_scene_event:<bare_name>`)
    /// 4. Payload size limit (≤ 4 KB)
    /// 5. Rate limit (sliding 1-second window)
    ///
    /// On success, returns `Ok(EmissionOutcome)` with the fully-prefixed event
    /// type ready for delivery to subscribers.
    ///
    /// `now` should be `Instant::now()` in production code; it is a parameter
    /// to allow deterministic unit testing.
    pub fn handle(
        &mut self,
        bare_name: &str,
        payload: Vec<u8>,
        now: Instant,
    ) -> EmissionResult {
        // ── Step 1 & 2: Validate bare name (checks format AND reserved prefixes) ──
        validate_bare_name(bare_name).map_err(|e| {
            use tze_hud_scene::events::naming::NamingError;
            match &e {
                NamingError::ReservedPrefix { prefix } => EmissionError::ReservedPrefix {
                    prefix: prefix.clone(),
                },
                _ => EmissionError::InvalidName {
                    detail: e.to_string(),
                },
            }
        })?;

        // ── Step 3: Capability check ────────────────────────────────────────────
        let required_cap = format!("{EMIT_CAPABILITY_PREFIX}{bare_name}");
        if !self.capabilities.contains(&required_cap) {
            return Err(EmissionError::CapabilityMissing {
                required: required_cap,
            });
        }

        // ── Step 4: Payload size limit ──────────────────────────────────────────
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err(EmissionError::PayloadTooLarge {
                actual: payload.len(),
                limit: MAX_PAYLOAD_BYTES,
            });
        }

        // ── Step 5: Rate limit ──────────────────────────────────────────────────
        self.rate_limiter.check_and_record(now).map_err(|()| EmissionError::RateLimitExceeded)?;

        // ── Accepted: build fully-prefixed event type ───────────────────────────
        let delivered_event_type = build_agent_event_type(&self.namespace, bare_name);

        Ok(EmissionOutcome {
            delivered_event_type,
            bare_name: bare_name.to_string(),
        })
    }

    /// Whether the agent holds the given capability.
    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.contains(&cap.to_string())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn caps(bare_names: &[&str]) -> Vec<String> {
        bare_names
            .iter()
            .map(|n| format!("emit_scene_event:{n}"))
            .collect()
    }

    fn ms_after(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    // ── Spec scenario: Capability-gated emission (spec lines 112-114) ─────────

    /// WHEN an agent WITHOUT `emit_scene_event:doorbell.ring` capability attempts
    /// to emit "doorbell.ring" THEN the runtime MUST reject the emission with an
    /// error in EmitSceneEventResult (spec line 113).
    #[test]
    fn emission_without_capability_rejected() {
        let mut handler = AgentEventHandler::new("doorbell_agent", vec![]);
        let result = handler.handle("doorbell.ring", b"{}".to_vec(), Instant::now());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_code(), "AGENT_EVENT_CAPABILITY_MISSING");
        match err {
            EmissionError::CapabilityMissing { required } => {
                assert_eq!(required, "emit_scene_event:doorbell.ring");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    // ── Spec scenario: Successful emission with namespace prefix (spec line 118) ─

    /// WHEN an agent with namespace "alarm_agent" and `emit_scene_event:fire.detected`
    /// capability emits "fire.detected" THEN the event MUST be delivered to subscribers
    /// with event_type "agent.alarm_agent.fire.detected" (spec line 118).
    #[test]
    fn successful_emission_produces_correct_event_type() {
        let mut handler =
            AgentEventHandler::new("alarm_agent", caps(&["fire.detected"]));
        let result = handler.handle("fire.detected", b"{}".to_vec(), Instant::now());
        assert!(result.is_ok(), "emission should be accepted: {result:?}");
        let outcome = result.unwrap();
        assert_eq!(outcome.delivered_event_type, "agent.alarm_agent.fire.detected");
        assert_eq!(outcome.bare_name, "fire.detected");
    }

    // ── Namespace prefixing ───────────────────────────────────────────────────

    #[test]
    fn namespace_prefixing_doorbell() {
        let mut handler =
            AgentEventHandler::new("doorbell_agent", caps(&["doorbell.ring"]));
        let outcome = handler
            .handle("doorbell.ring", b"{}".to_vec(), Instant::now())
            .unwrap();
        assert_eq!(outcome.delivered_event_type, "agent.doorbell_agent.doorbell.ring");
    }

    // ── Spec scenario: Payload size limit (spec lines 120-122) ───────────────

    /// WHEN an agent emits an event with a payload exceeding 4KB THEN the
    /// runtime MUST reject the emission (spec line 122).
    #[test]
    fn payload_over_4kb_rejected() {
        let mut handler =
            AgentEventHandler::new("big_agent", caps(&["sensor.data"]));
        let oversized = vec![0u8; MAX_PAYLOAD_BYTES + 1];
        let result = handler.handle("sensor.data", oversized, Instant::now());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_code(), "AGENT_EVENT_PAYLOAD_TOO_LARGE");
        match err {
            EmissionError::PayloadTooLarge { actual, limit } => {
                assert_eq!(actual, MAX_PAYLOAD_BYTES + 1);
                assert_eq!(limit, MAX_PAYLOAD_BYTES);
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    /// Exactly 4096 bytes is accepted.
    #[test]
    fn payload_exactly_4kb_accepted() {
        let mut handler =
            AgentEventHandler::new("sensor_agent", caps(&["sensor.data"]));
        let exactly_4k = vec![0u8; MAX_PAYLOAD_BYTES];
        assert!(handler.handle("sensor.data", exactly_4k, Instant::now()).is_ok());
    }

    // ── Spec scenario: Rate limit enforcement (spec lines 131-133) ───────────

    /// WHEN an agent emits 11 events within a 1-second sliding window (default
    /// limit: 10/s) THEN the 11th event MUST be rejected with
    /// AGENT_EVENT_RATE_EXCEEDED (spec line 133).
    #[test]
    fn eleventh_event_in_window_rejected() {
        let base = Instant::now();
        let mut handler =
            AgentEventHandler::new("chatty", caps(&["sensor.ping"]));

        for i in 0..10 {
            let t = ms_after(base, i * 50);
            assert!(
                handler.handle("sensor.ping", b"{}".to_vec(), t).is_ok(),
                "event {i} should be accepted"
            );
        }

        // 11th event — must be rejected.
        let t11 = ms_after(base, 550);
        let result = handler.handle("sensor.ping", b"{}".to_vec(), t11);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code(), "AGENT_EVENT_RATE_EXCEEDED");
    }

    // ── Reserved prefix rejection ─────────────────────────────────────────────

    /// WHEN an agent emits with bare name "system.fake" THEN rejected with
    /// AGENT_EVENT_RESERVED_PREFIX (spec line 46).
    #[test]
    fn system_prefix_rejected() {
        let mut handler = AgentEventHandler::new("bad_agent", vec![
            "emit_scene_event:system.fake".to_string(),
        ]);
        let result = handler.handle("system.fake", b"{}".to_vec(), Instant::now());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code(), "AGENT_EVENT_RESERVED_PREFIX");
    }

    /// WHEN an agent emits with bare name "scene.impersonate" THEN rejected.
    #[test]
    fn scene_prefix_rejected() {
        let mut handler = AgentEventHandler::new("bad_agent", vec![
            "emit_scene_event:scene.impersonate".to_string(),
        ]);
        let result = handler.handle("scene.impersonate", b"{}".to_vec(), Instant::now());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code(), "AGENT_EVENT_RESERVED_PREFIX");
    }

    // ── Bare name validation ──────────────────────────────────────────────────

    /// Uppercase bare names rejected.
    #[test]
    fn uppercase_bare_name_rejected() {
        let mut handler = AgentEventHandler::new("agent", vec![
            "emit_scene_event:Doorbell.Ring".to_string(),
        ]);
        let result = handler.handle("Doorbell.Ring", b"{}".to_vec(), Instant::now());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code(), "AGENT_EVENT_INVALID_NAME");
    }

    /// Bare name with no dot (single segment) rejected.
    #[test]
    fn single_segment_bare_name_rejected() {
        let mut handler =
            AgentEventHandler::new("agent", vec!["emit_scene_event:doorbell".to_string()]);
        let result = handler.handle("doorbell", b"{}".to_vec(), Instant::now());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code(), "AGENT_EVENT_INVALID_NAME");
    }

    /// Valid multi-segment bare name with underscore accepted.
    #[test]
    fn valid_bare_name_with_underscore() {
        let mut handler =
            AgentEventHandler::new("agent", caps(&["weather.update_summary"]));
        assert!(handler
            .handle("weather.update_summary", b"{}".to_vec(), Instant::now())
            .is_ok());
    }

    // ── Error code/message helpers ────────────────────────────────────────────

    #[test]
    fn error_code_strings_are_correct() {
        assert_eq!(
            EmissionError::RateLimitExceeded.error_code(),
            "AGENT_EVENT_RATE_EXCEEDED"
        );
        assert_eq!(
            EmissionError::PayloadTooLarge { actual: 5000, limit: 4096 }.error_code(),
            "AGENT_EVENT_PAYLOAD_TOO_LARGE"
        );
        assert_eq!(
            EmissionError::CapabilityMissing { required: "emit_scene_event:x.y".to_string() }
                .error_code(),
            "AGENT_EVENT_CAPABILITY_MISSING"
        );
        assert_eq!(
            EmissionError::InvalidName { detail: "bad".to_string() }.error_code(),
            "AGENT_EVENT_INVALID_NAME"
        );
        assert_eq!(
            EmissionError::ReservedPrefix { prefix: "system.".to_string() }.error_code(),
            "AGENT_EVENT_RESERVED_PREFIX"
        );
    }
}
