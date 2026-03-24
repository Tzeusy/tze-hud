//! Authentication and capability negotiation for the session handshake.
//!
//! Implements RFC 0005 §1.4 (Authentication) and the capability gating
//! policy described in RFC 0005 §5.3. This module is responsible for:
//!
//! - Evaluating `AuthCredential` during `SessionInit` / `SessionResume`
//! - Filtering requested capabilities against an agent authorization policy
//! - Evaluating mid-session `CapabilityRequest` against the same policy
//! - Filtering initial subscriptions by granted capabilities
//!
//! # V1 Auth Implementations
//!
//! Per RFC 0005 §1.4 and the v1-mandatory scope, two credential types are
//! fully implemented:
//!
//! - `PreSharedKeyCredential` — matched against the server PSK.
//! - `LocalSocketCredential` — accepted unconditionally on loopback.
//!
//! `OauthTokenCredential` and `MtlsCredential` are schema-defined
//! (proto messages exist) but their implementations are v1-reserved; they
//! are rejected with `AUTH_FAILED` until a future release enables them.

use crate::proto::session::{
    auth_credential::Credential, AuthCredential,
};

// Constant-time byte-level equality to resist timing side-channels.
// `subtle` is not in scope for v1; we use a manual xor-fold that is
// branch-free across the key bytes and a fixed-time length check.
//
// This is NOT a cryptographic HMAC; for v1 PSK the surface is local
// gRPC only. Replace with subtle::ConstantTimeEq when the crate is added.
fn ct_eq_str(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ─── Subscription capability requirements ────────────────────────────────────

/// Returns the capability required to subscribe to the given subscription
/// category (RFC 0005 §7.1). Returns `None` for categories that are always
/// allowed regardless of capabilities (DEGRADATION_NOTICES, LEASE_CHANGES).
pub fn required_capability_for_subscription(category: &str) -> Option<&'static str> {
    match category {
        "SCENE_TOPOLOGY"       => Some("read_scene_topology"),
        "INPUT_EVENTS"         => Some("access_input_events"),
        "FOCUS_EVENTS"         => Some("access_input_events"),
        "ZONE_EVENTS"          => Some("publish_zone"),  // publish_zone:<zone> in full spec
        "TELEMETRY_FRAMES"     => Some("read_telemetry"),
        "ATTENTION_EVENTS"     => Some("read_scene_topology"),
        "AGENT_EVENTS"         => Some("subscribe_scene_events"),
        // Always subscribed; capability not required:
        "DEGRADATION_NOTICES"  => None,
        "LEASE_CHANGES"        => None,
        _ => None, // Unknown categories: allow by default (forward compat)
    }
}

// ─── Credential evaluation ────────────────────────────────────────────────────

/// Result of an authentication attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResult {
    /// Authentication succeeded.
    Accepted,
    /// Authentication failed. The reason string is sent in `SessionError`.
    Failed(String),
    /// The credential type is not yet implemented (v1-reserved).
    Unimplemented(String),
}

/// Evaluate a structured `AuthCredential` against the server configuration.
///
/// `psk` is the pre-shared key configured on the server.
pub fn evaluate_auth_credential(credential: &AuthCredential, psk: &str) -> AuthResult {
    match &credential.credential {
        Some(Credential::PreSharedKey(cred)) => {
            // Use branch-free comparison to resist timing side-channels.
            if ct_eq_str(&cred.key, psk) {
                AuthResult::Accepted
            } else {
                AuthResult::Failed("pre-shared key mismatch".to_string())
            }
        }
        Some(Credential::LocalSocket(_cred)) => {
            // V1: local socket connections are accepted unconditionally.
            // In a production system we would verify the PID/socket path.
            AuthResult::Accepted
        }
        Some(Credential::OauthToken(_)) => {
            // v1-reserved: OauthTokenCredential schema exists but is not implemented.
            AuthResult::Unimplemented(
                "OauthTokenCredential is not implemented in v1; use PreSharedKeyCredential"
                    .to_string(),
            )
        }
        Some(Credential::Mtls(_)) => {
            // v1-reserved: MtlsCredential schema exists but is not implemented.
            AuthResult::Unimplemented(
                "MtlsCredential is not implemented in v1; use PreSharedKeyCredential".to_string(),
            )
        }
        None => {
            // Empty AuthCredential: treat as "no credential provided" — fail auth.
            AuthResult::Failed("no credential provided in AuthCredential".to_string())
        }
    }
}

/// Authenticate from a `SessionInit` message.
///
/// Checks the structured `auth_credential` field first; falls back to the
/// deprecated `pre_shared_key` string field for backward compatibility with
/// agents built before the `AuthCredential` oneof was added.
pub fn authenticate_session_init(
    auth_credential: Option<&AuthCredential>,
    legacy_psk: &str,
    server_psk: &str,
) -> AuthResult {
    // If a structured credential is provided, use it.
    if let Some(cred) = auth_credential {
        if cred.credential.is_some() {
            return evaluate_auth_credential(cred, server_psk);
        }
    }

    // Fall back to the deprecated plain-string PSK field.
    // Use branch-free comparison to resist timing side-channels.
    if ct_eq_str(legacy_psk, server_psk) {
        AuthResult::Accepted
    } else {
        AuthResult::Failed("invalid pre-shared key".to_string())
    }
}

// ─── Protocol version negotiation (RFC 0005 §4.1) ────────────────────────────

/// Runtime's supported version range.
/// `version = major * 1000 + minor`.
pub const RUNTIME_MIN_VERSION: u32 = 1000; // v1.0
pub const RUNTIME_MAX_VERSION: u32 = 1001; // v1.1

/// Negotiate the protocol version between agent and runtime.
///
/// Returns the highest mutually supported version, or `Err` with an
/// `UNSUPPORTED_PROTOCOL_VERSION` message if no mutual version exists.
///
/// If the agent sends `min=0, max=0` (unset), we treat it as `min=1000, max=1000`
/// (v1.0 only, backward compatible).
pub fn negotiate_version(
    agent_min: u32,
    agent_max: u32,
) -> Result<u32, String> {
    // Treat 0 (unset) as v1.0 for backward compatibility.
    let a_min = if agent_min == 0 { RUNTIME_MIN_VERSION } else { agent_min };
    let a_max = if agent_max == 0 { RUNTIME_MIN_VERSION } else { agent_max };

    // Find the highest version in the intersection of [a_min, a_max] and [RUNTIME_MIN, RUNTIME_MAX].
    let low  = a_min.max(RUNTIME_MIN_VERSION);
    let high = a_max.min(RUNTIME_MAX_VERSION);

    if low > high {
        Err(format!(
            "no mutual protocol version: agent supports {a_min}-{a_max}, \
             runtime supports {RUNTIME_MIN_VERSION}-{RUNTIME_MAX_VERSION}"
        ))
    } else {
        Ok(high) // pick the highest mutual version
    }
}

// ─── Capability policy (RFC 0005 §5.3) ───────────────────────────────────────

/// Authorization policy for a single agent.
///
/// In v1 the policy is derived from the agent's registration (configured
/// PSK-based identity → allowed capability set). In a future release this
/// will consult a per-agent config file or dynamic approval flow.
///
/// # V1 policy rules
///
/// - Any capability listed in `allowed` is grantable.
/// - Any capability NOT in `allowed` is denied.
/// - Partial grants are denied entirely (RFC 0005 §5.3 scenario 4).
#[derive(Debug, Clone)]
pub struct CapabilityPolicy {
    /// The full set of capabilities this agent is authorized to hold.
    /// An empty set means the agent has no special capabilities (guest).
    allowed: Vec<String>,
}

impl CapabilityPolicy {
    /// Create a policy that allows exactly the given set of capabilities.
    pub fn new(allowed: Vec<String>) -> Self {
        Self { allowed }
    }

    /// Unrestricted policy — allows any capability.
    ///
    /// Used for PSK-authenticated agents in v1 where the PSK holder is
    /// implicitly trusted for all capabilities.
    pub fn unrestricted() -> Self {
        // Sentinel: `"*"` in `allowed` indicates an allow-all policy.
        // `is_unrestricted()` and `permits()` both rely on this `"*"` marker.
        Self { allowed: vec!["*".to_string()] }
    }

    /// Guest policy — no capabilities granted.
    pub fn guest() -> Self {
        Self { allowed: Vec::new() }
    }

    /// Returns `true` if this policy is unrestricted (permits any capability).
    pub fn is_unrestricted(&self) -> bool {
        self.allowed.iter().any(|a| a == "*")
    }

    /// Returns `true` if this policy grants the given capability.
    fn permits(&self, capability: &str) -> bool {
        self.allowed.iter().any(|a| a == "*" || a == capability)
    }

    /// Evaluate a capability grant request (RFC 0005 §5.3).
    ///
    /// Returns `Ok(Vec<String>)` with the granted capabilities on success, or
    /// `Err(Vec<String>)` listing the unauthorized capabilities if any are
    /// denied (the entire request is denied on any partial failure).
    pub fn evaluate_capability_request(
        &self,
        requested: &[String],
    ) -> Result<Vec<String>, Vec<String>> {
        let denied: Vec<String> = requested
            .iter()
            .filter(|cap| !self.permits(cap))
            .cloned()
            .collect();

        if denied.is_empty() {
            Ok(requested.to_vec())
        } else {
            // Deny the entire request on partial failure (RFC 0005 §5.3 scenario 4).
            Err(denied)
        }
    }

    /// Filter a set of requested capabilities into (granted, denied) lists
    /// for reporting in `SessionEstablished`.
    ///
    /// Unlike `evaluate_capability_request`, this does NOT deny the entire
    /// batch on partial failure; it partitions the set. Used only at handshake
    /// where individual grants/denials are reported separately.
    pub fn partition_capabilities(
        &self,
        requested: &[String],
    ) -> (Vec<String>, Vec<String>) {
        let mut granted = Vec::new();
        let mut denied = Vec::new();
        for cap in requested {
            if self.permits(cap) {
                granted.push(cap.clone());
            } else {
                denied.push(cap.clone());
            }
        }
        (granted, denied)
    }

    /// Derive a capability policy for a PSK-authenticated agent.
    ///
    /// In v1, a valid PSK grants unrestricted access. Future versions will
    /// consult per-agent registration files.
    pub fn for_psk_agent() -> Self {
        Self::unrestricted()
    }
}

// ─── Subscription filtering ───────────────────────────────────────────────────

/// Filter an agent's requested initial subscriptions against their granted capabilities.
///
/// Returns `(active, denied)` where:
/// - `active` are subscriptions the agent is allowed to receive.
/// - `denied` are subscriptions that require a capability the agent wasn't granted.
pub fn filter_subscriptions(
    requested: &[String],
    granted_capabilities: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut active = Vec::new();
    let mut denied = Vec::new();

    for sub in requested {
        match required_capability_for_subscription(sub) {
            None => {
                // No capability required (or always subscribed) — allow.
                active.push(sub.clone());
            }
            Some(required) => {
                let has_cap = granted_capabilities.iter().any(|c| c == "*" || c == required);
                if has_cap {
                    active.push(sub.clone());
                } else {
                    denied.push(sub.clone());
                }
            }
        }
    }

    (active, denied)
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::session::{
        auth_credential::Credential,
        AuthCredential, PreSharedKeyCredential, LocalSocketCredential,
    };

    fn psk_credential(key: &str) -> AuthCredential {
        AuthCredential {
            credential: Some(Credential::PreSharedKey(PreSharedKeyCredential {
                key: key.to_string(),
            })),
        }
    }

    fn local_socket_credential() -> AuthCredential {
        AuthCredential {
            credential: Some(Credential::LocalSocket(LocalSocketCredential {
                socket_path: "/run/tze_hud.sock".to_string(),
                pid_hint: "1234".to_string(),
            })),
        }
    }

    // ── Auth credential tests ──────────────────────────────────────────────────

    #[test]
    fn test_psk_credential_success() {
        let cred = psk_credential("secret");
        assert_eq!(evaluate_auth_credential(&cred, "secret"), AuthResult::Accepted);
    }

    #[test]
    fn test_psk_credential_failure() {
        let cred = psk_credential("wrong");
        match evaluate_auth_credential(&cred, "secret") {
            AuthResult::Failed(_) => {}
            other => panic!("Expected Failed, got: {other:?}"),
        }
    }

    #[test]
    fn test_local_socket_credential_accepted() {
        let cred = local_socket_credential();
        assert_eq!(evaluate_auth_credential(&cred, "secret"), AuthResult::Accepted);
    }

    #[test]
    fn test_oauth_credential_unimplemented() {
        use crate::proto::session::OauthTokenCredential;
        let cred = AuthCredential {
            credential: Some(Credential::OauthToken(OauthTokenCredential {
                bearer_token: "token".to_string(),
                token_type: "Bearer".to_string(),
            })),
        };
        match evaluate_auth_credential(&cred, "secret") {
            AuthResult::Unimplemented(_) => {}
            other => panic!("Expected Unimplemented, got: {other:?}"),
        }
    }

    #[test]
    fn test_mtls_credential_unimplemented() {
        use crate::proto::session::MtlsCredential;
        let cred = AuthCredential {
            credential: Some(Credential::Mtls(MtlsCredential {
                client_certificate_der: vec![1, 2, 3],
                expected_san: "test".to_string(),
            })),
        };
        match evaluate_auth_credential(&cred, "secret") {
            AuthResult::Unimplemented(_) => {}
            other => panic!("Expected Unimplemented, got: {other:?}"),
        }
    }

    #[test]
    fn test_empty_credential_fails() {
        let cred = AuthCredential { credential: None };
        match evaluate_auth_credential(&cred, "secret") {
            AuthResult::Failed(_) => {}
            other => panic!("Expected Failed, got: {other:?}"),
        }
    }

    // ── authenticate_session_init tests ───────────────────────────────────────

    #[test]
    fn test_session_init_structured_cred_takes_precedence() {
        let cred = psk_credential("correct");
        // Even with wrong legacy PSK, structured cred with correct key should pass
        assert_eq!(
            authenticate_session_init(Some(&cred), "wrong-legacy", "correct"),
            AuthResult::Accepted
        );
    }

    #[test]
    fn test_session_init_legacy_psk_fallback() {
        // No structured credential → falls back to legacy pre_shared_key field
        assert_eq!(
            authenticate_session_init(None, "correct", "correct"),
            AuthResult::Accepted
        );
    }

    #[test]
    fn test_session_init_legacy_psk_fallback_failure() {
        match authenticate_session_init(None, "wrong", "correct") {
            AuthResult::Failed(_) => {}
            other => panic!("Expected Failed, got: {other:?}"),
        }
    }

    #[test]
    fn test_session_init_empty_structured_cred_uses_legacy() {
        // AuthCredential with no credential variant set → fall back to legacy field
        let empty_cred = AuthCredential { credential: None };
        assert_eq!(
            authenticate_session_init(Some(&empty_cred), "correct", "correct"),
            AuthResult::Accepted
        );
    }

    // ── Version negotiation tests ─────────────────────────────────────────────

    #[test]
    fn test_version_negotiation_success() {
        // Agent supports 1000-1001, runtime supports 1000-1001 → pick 1001
        assert_eq!(negotiate_version(1000, 1001), Ok(1001));
    }

    #[test]
    fn test_version_negotiation_exact_match() {
        assert_eq!(negotiate_version(1000, 1000), Ok(1000));
    }

    #[test]
    fn test_version_negotiation_no_overlap() {
        // Agent supports 2000-2001, runtime supports 1000-1001 → fail
        assert!(negotiate_version(2000, 2001).is_err());
    }

    #[test]
    fn test_version_negotiation_unset_treated_as_v1() {
        // min=0, max=0 → treated as 1000-1000 → pick 1000
        assert_eq!(negotiate_version(0, 0), Ok(1000));
    }

    #[test]
    fn test_version_negotiation_agent_below_runtime() {
        // Agent only supports 999 which is below RUNTIME_MIN_VERSION=1000
        assert!(negotiate_version(900, 999).is_err());
    }

    // ── Capability policy tests ────────────────────────────────────────────────

    #[test]
    fn test_policy_unrestricted_allows_any() {
        let policy = CapabilityPolicy::unrestricted();
        assert!(policy.permits("create_tiles"));
        assert!(policy.permits("read_telemetry"));
        assert!(policy.permits("overlay_privileges"));
    }

    #[test]
    fn test_policy_guest_denies_all() {
        let policy = CapabilityPolicy::guest();
        assert!(!policy.permits("create_tiles"));
        assert!(!policy.permits("read_telemetry"));
    }

    #[test]
    fn test_policy_specific_allows_listed() {
        let policy = CapabilityPolicy::new(vec!["read_telemetry".to_string()]);
        assert!(policy.permits("read_telemetry"));
        assert!(!policy.permits("overlay_privileges"));
    }

    #[test]
    fn test_capability_request_all_authorized() {
        let policy = CapabilityPolicy::unrestricted();
        let result = policy.evaluate_capability_request(&[
            "read_telemetry".to_string(),
        ]);
        assert_eq!(result, Ok(vec!["read_telemetry".to_string()]));
    }

    #[test]
    fn test_capability_request_unauthorized_denied() {
        let policy = CapabilityPolicy::guest();
        let result = policy.evaluate_capability_request(&[
            "overlay_privileges".to_string(),
        ]);
        assert!(result.is_err());
    }

    /// Scenario: Partial grant of mixed capabilities is denied entirely
    /// (RFC 0005 §5.3 scenario 4)
    #[test]
    fn test_capability_request_partial_grant_denied_entirely() {
        let policy = CapabilityPolicy::new(vec!["read_telemetry".to_string()]);
        // read_telemetry authorized, overlay_privileges not — deny entire request
        let result = policy.evaluate_capability_request(&[
            "read_telemetry".to_string(),
            "overlay_privileges".to_string(),
        ]);
        match result {
            Err(denied) => {
                assert!(denied.contains(&"overlay_privileges".to_string()));
                // read_telemetry should NOT appear in the denied list
                // (the error lists only the unauthorized ones, not the whole request)
            }
            Ok(_) => panic!("Expected denial for mixed capabilities"),
        }
    }

    #[test]
    fn test_partition_capabilities() {
        let policy = CapabilityPolicy::new(vec![
            "read_telemetry".to_string(),
            "create_tiles".to_string(), // canonical: plural
        ]);
        let (granted, denied) = policy.partition_capabilities(&[
            "read_telemetry".to_string(),
            "overlay_privileges".to_string(),
            "create_tiles".to_string(), // canonical: plural
        ]);
        assert_eq!(granted, vec!["read_telemetry", "create_tiles"]);
        assert_eq!(denied, vec!["overlay_privileges"]);
    }

    // ── Subscription filtering tests ───────────────────────────────────────────

    /// Scenario: Denied subscription for missing capability
    /// (RFC 0005 §7.1 scenario)
    #[test]
    fn test_subscription_denied_without_capability() {
        let (active, denied) = filter_subscriptions(
            &["INPUT_EVENTS".to_string()],
            &[/* no access_input_events */],
        );
        assert!(active.is_empty());
        assert_eq!(denied, vec!["INPUT_EVENTS"]);
    }

    #[test]
    fn test_subscription_allowed_with_capability() {
        let (active, denied) = filter_subscriptions(
            &["INPUT_EVENTS".to_string()],
            &["access_input_events".to_string()],
        );
        assert_eq!(active, vec!["INPUT_EVENTS"]);
        assert!(denied.is_empty());
    }

    #[test]
    fn test_subscription_always_allowed() {
        // DEGRADATION_NOTICES and LEASE_CHANGES need no capability
        let (active, denied) = filter_subscriptions(
            &["DEGRADATION_NOTICES".to_string(), "LEASE_CHANGES".to_string()],
            &[/* no capabilities */],
        );
        assert_eq!(active.len(), 2);
        assert!(denied.is_empty());
    }

    #[test]
    fn test_subscription_unrestricted_allows_all() {
        let (active, denied) = filter_subscriptions(
            &[
                "SCENE_TOPOLOGY".to_string(),
                "INPUT_EVENTS".to_string(),
                "TELEMETRY_FRAMES".to_string(),
            ],
            &["*".to_string()],
        );
        assert_eq!(active.len(), 3);
        assert!(denied.is_empty());
    }

    #[test]
    fn test_subscription_mixed_capabilities() {
        let (active, denied) = filter_subscriptions(
            &[
                "SCENE_TOPOLOGY".to_string(),   // requires read_scene_topology
                "INPUT_EVENTS".to_string(),      // requires access_input_events
                "DEGRADATION_NOTICES".to_string(), // always allowed
            ],
            &["read_scene_topology".to_string()], // only has read_scene_topology
        );
        assert!(active.contains(&"SCENE_TOPOLOGY".to_string()));
        assert!(active.contains(&"DEGRADATION_NOTICES".to_string()));
        assert!(denied.contains(&"INPUT_EVENTS".to_string()));
    }
}
