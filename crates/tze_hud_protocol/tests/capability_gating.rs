//! Capability gating tests.
//!
//! Tests guest vs resident tool access, subscription category-to-capability
//! mapping, and fine-grained capability enforcement.
//!
//! Based on session-protocol/spec.md lines 487-510 and auth.rs CapabilityPolicy.
//!
//! Test count target: ≥10 tests.

use tze_hud_protocol::auth::{
    CapabilityPolicy, authenticate_session_init, negotiate_version,
    RUNTIME_MIN_VERSION, RUNTIME_MAX_VERSION,
};
use tze_hud_protocol::proto::session::{
    AuthCredential, PreSharedKeyCredential, LocalSocketCredential,
};
use tze_hud_protocol::proto::session::auth_credential::Credential;
use tze_hud_protocol::auth::AuthResult;

// ─── Capability Policy ───────────────────────────────────────────────────────

/// WHEN agent has resident_mcp capability THEN evaluate_capability_request succeeds.
#[test]
fn resident_mcp_capability_granted_when_allowed() {
    let policy = CapabilityPolicy::new(vec!["resident_mcp".to_string()]);
    let result = policy.evaluate_capability_request(&["resident_mcp".to_string()]);
    assert!(result.is_ok(), "resident_mcp must be granted when in allowed set");
    assert_eq!(result.unwrap(), vec!["resident_mcp"]);
}

/// WHEN agent requests capability not in allowed set THEN entire request denied.
#[test]
fn capability_request_denied_when_not_allowed() {
    let policy = CapabilityPolicy::new(vec!["read_scene_topology".to_string()]);
    let result = policy.evaluate_capability_request(&["resident_mcp".to_string()]);
    assert!(result.is_err(), "resident_mcp must be denied when not in allowed set");
    let denied = result.unwrap_err();
    assert!(denied.contains(&"resident_mcp".to_string()));
}

/// WHEN agent is a guest (no capabilities) THEN resident tool request denied.
#[test]
fn guest_policy_denies_all_capabilities() {
    let policy = CapabilityPolicy::guest();
    let result = policy.evaluate_capability_request(&[
        "resident_mcp".to_string(),
        "create_tile".to_string(),
    ]);
    assert!(result.is_err(), "guest policy must deny all capability requests");
}

/// WHEN unrestricted policy THEN any capability request succeeds.
#[test]
fn unrestricted_policy_allows_any_capability() {
    let policy = CapabilityPolicy::unrestricted();
    let result = policy.evaluate_capability_request(&[
        "resident_mcp".to_string(),
        "read_scene_topology".to_string(),
        "access_input_events".to_string(),
        "publish_zone:subtitle".to_string(),
    ]);
    assert!(result.is_ok(), "unrestricted policy must allow any capability");
    assert!(policy.is_unrestricted());
}

/// WHEN partial capability request (some allowed, some not) THEN entire batch denied.
#[test]
fn partial_capability_request_entirely_denied() {
    // RFC 0005 §5.3 scenario 4: partial grants are denied entirely
    let policy = CapabilityPolicy::new(vec!["read_scene_topology".to_string()]);
    let result = policy.evaluate_capability_request(&[
        "read_scene_topology".to_string(),
        "resident_mcp".to_string(), // not in allowed set
    ]);
    assert!(result.is_err(), "partial capability request must be entirely denied");
    let denied = result.unwrap_err();
    assert!(denied.contains(&"resident_mcp".to_string()),
        "denied list must include the unauthorized capability");
}

/// WHEN guest policy created THEN is_unrestricted returns false.
#[test]
fn guest_policy_is_not_unrestricted() {
    let policy = CapabilityPolicy::guest();
    assert!(!policy.is_unrestricted());
}

/// WHEN fine-grained publish_zone capability with specific zone name THEN allowed.
#[test]
fn fine_grained_publish_zone_capability_matches_specific_zone() {
    let policy = CapabilityPolicy::new(vec!["publish_zone:subtitle".to_string()]);
    let result = policy.evaluate_capability_request(&["publish_zone:subtitle".to_string()]);
    assert!(result.is_ok(), "publish_zone:subtitle must be granted when in allowed set");
}

/// WHEN agent requests publish_zone:other with only publish_zone:subtitle THEN denied.
#[test]
fn fine_grained_publish_zone_capability_does_not_match_other_zone() {
    let policy = CapabilityPolicy::new(vec!["publish_zone:subtitle".to_string()]);
    let result = policy.evaluate_capability_request(&["publish_zone:notification".to_string()]);
    assert!(result.is_err(),
        "publish_zone:notification must not be granted when only publish_zone:subtitle is allowed");
}

// ─── Authentication ───────────────────────────────────────────────────────────

/// WHEN PSK matches server PSK THEN authentication succeeds.
#[test]
fn psk_authentication_succeeds_with_correct_key() {
    let server_psk = "secret-key-abc";
    let cred = AuthCredential {
        credential: Some(Credential::PreSharedKey(PreSharedKeyCredential {
            key: "secret-key-abc".to_string(),
        })),
    };
    let result = authenticate_session_init(Some(&cred), "", server_psk);
    assert_eq!(result, AuthResult::Accepted);
}

/// WHEN PSK does not match server PSK THEN authentication fails.
#[test]
fn psk_authentication_fails_with_wrong_key() {
    let server_psk = "secret-key-abc";
    let cred = AuthCredential {
        credential: Some(Credential::PreSharedKey(PreSharedKeyCredential {
            key: "wrong-key".to_string(),
        })),
    };
    let result = authenticate_session_init(Some(&cred), "", server_psk);
    assert!(matches!(result, AuthResult::Failed(_)),
        "wrong PSK must fail authentication");
}

/// WHEN local socket credential provided THEN authentication accepted unconditionally (v1).
#[test]
fn local_socket_authentication_accepted_unconditionally() {
    let cred = AuthCredential {
        credential: Some(Credential::LocalSocket(LocalSocketCredential {
            socket_path: "/run/tze_hud.sock".to_string(),
            pid_hint: "1234".to_string(),
        })),
    };
    let result = authenticate_session_init(Some(&cred), "", "server-psk");
    assert_eq!(result, AuthResult::Accepted,
        "local socket credential must be accepted unconditionally on loopback");
}

/// WHEN legacy PSK field used (no auth_credential) THEN falls back to legacy check.
#[test]
fn legacy_psk_fallback_accepted() {
    let server_psk = "legacy-key";
    // No auth_credential — falls back to legacy pre_shared_key string
    let result = authenticate_session_init(None, "legacy-key", server_psk);
    assert_eq!(result, AuthResult::Accepted);
}

/// WHEN legacy PSK field wrong THEN fails.
#[test]
fn legacy_psk_fallback_rejected() {
    let server_psk = "legacy-key";
    let result = authenticate_session_init(None, "wrong-legacy-key", server_psk);
    assert!(matches!(result, AuthResult::Failed(_)));
}

/// WHEN empty auth credential provided THEN fails.
#[test]
fn empty_auth_credential_fails() {
    let cred = AuthCredential { credential: None };
    let result = authenticate_session_init(Some(&cred), "", "server-psk");
    assert!(matches!(result, AuthResult::Failed(_)),
        "empty AuthCredential must fail authentication");
}

// ─── Protocol version negotiation ────────────────────────────────────────────

/// WHEN agent supports v1.0 THEN negotiation succeeds at v1.0.
#[test]
fn version_negotiation_succeeds_at_v1_0() {
    let version = negotiate_version(1000, 1000);
    assert!(version.is_ok());
    assert_eq!(version.unwrap(), 1000);
}

/// WHEN agent supports v1.0-v1.1 THEN negotiation picks highest (v1.1).
#[test]
fn version_negotiation_picks_highest_mutual_version() {
    let version = negotiate_version(1000, 1001);
    assert!(version.is_ok());
    assert_eq!(version.unwrap(), RUNTIME_MAX_VERSION);
}

/// WHEN agent supports v0.9 only THEN negotiation fails.
#[test]
fn version_negotiation_fails_when_no_intersection() {
    // Agent only supports pre-v1.0
    let version = negotiate_version(900, 999);
    assert!(version.is_err(),
        "version negotiation must fail when agent range does not intersect runtime range");
}

/// WHEN agent sends min=0, max=0 (unset) THEN treated as v1.0.
#[test]
fn version_negotiation_treats_zero_as_v1_0() {
    let version = negotiate_version(0, 0);
    assert!(version.is_ok());
    assert_eq!(version.unwrap(), RUNTIME_MIN_VERSION);
}

/// WHEN agent supports v1.2+ only THEN negotiation fails (runtime max is 1.1).
#[test]
fn version_negotiation_fails_when_agent_requires_newer() {
    // Agent requires v1.2+
    let version = negotiate_version(1002, 1010);
    assert!(version.is_err(),
        "negotiation must fail when agent minimum exceeds runtime maximum");
}
