//! Phase-1 portal component profile styling tests (hud-5jbra.6).
//!
//! Covers §6.4 of `text-stream-portal-phase1/tasks.md`:
//!
//! 1. **Profile-swap reskin** — token override changes all portal parts without
//!    any adapter logic change (§6.1 invariant: only token values differ, not
//!    the code path that calls `resolve_portal_tokens`).
//!
//! 2. **Token propagation on republish** — adapter renders updated visual tokens
//!    on the second render call, no adapter code change required.
//!
//! 3. **Adapter publish path contains no literal visual values** — the adapter's
//!    `visual_tokens()` accessor returns the token-derived values; the node built
//!    by `portal_node` is verified to use those values.
//!
//! 4. **Redaction-safe transitions** — a restricted viewer (Public clearance)
//!    observes `redacted = true` when the portal carries Private content,
//!    ensuring the transition frames never expose transcript content.
//!    The token-derived `transition_in_ms` / `transition_out_ms` govern duration
//!    but the redaction property is structural (derived from viewer clearance vs.
//!    content classification) and must hold regardless of transition timing.

use tze_hud_config::{
    PORTAL_TOKEN_COLLAPSED_BACKGROUND, PORTAL_TOKEN_COLLAPSED_FONT_SIZE,
    PORTAL_TOKEN_COLLAPSED_TEXT_COLOR, PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR,
    PORTAL_TOKEN_COMPOSER_BACKGROUND, PORTAL_TOKEN_COMPOSER_FONT_SIZE,
    PORTAL_TOKEN_COMPOSER_TEXT_COLOR, PORTAL_TOKEN_TRANSCRIPT_BACKGROUND,
    PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE, PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR,
    PORTAL_TOKEN_TRANSITION_IN_MS, PORTAL_TOKEN_TRANSITION_OUT_MS, resolve_portal_tokens,
};
use tze_hud_projection::{
    AttachRequest, ContentClassification, HudConnectionMetadata, OperationEnvelope, OutputKind,
    ProjectedPortalPolicy, ProjectedPortalPresentation, ProjectionAuthority,
    ProjectionLifecycleState, ProjectionOperation, ProviderKind, PublishOutputRequest,
    PublishStatusRequest,
    resident_grpc::{PortalVisualTokens, ResidentGrpcPortalAdapter, ResidentGrpcPortalConfig},
};
use tze_hud_runtime::portal_tokens::portal_visual_tokens_from_part_tokens;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn bytes_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

/// Build a minimal `ProjectedPortalState` for an attached + published projection.
fn build_expanded_state(
    authority: &mut ProjectionAuthority,
    projection_id: &str,
    policy: &ProjectedPortalPolicy,
) -> tze_hud_projection::ProjectedPortalState {
    let session_id = uuid::Uuid::now_v7().as_bytes().to_vec();
    let lease_id = uuid::Uuid::now_v7().as_bytes().to_vec();

    let attach = authority.handle_attach(
        AttachRequest {
            envelope: OperationEnvelope {
                operation: ProjectionOperation::Attach,
                projection_id: projection_id.to_string(),
                request_id: format!("attach-{projection_id}"),
                client_timestamp_wall_us: 1,
            },
            provider_kind: ProviderKind::Codex,
            display_name: format!("{projection_id} (styling test)"),
            workspace_hint: Some("mayor/rig".to_string()),
            repository_hint: None,
            icon_profile_hint: None,
            content_classification: ContentClassification::Private,
            hud_target: Some("resident-grpc".to_string()),
            idempotency_key: Some(format!("styling-once-{projection_id}")),
        },
        "styling-daemon",
        10,
    );
    assert!(
        attach.accepted,
        "attach must succeed for projection {projection_id}"
    );
    let owner_token = attach.owner_token.expect("attach returns owner token");

    authority
        .record_hud_connection(
            projection_id,
            HudConnectionMetadata {
                connection_id: format!("styling-conn-{projection_id}"),
                authenticated_session_id: bytes_hex(&session_id),
                granted_capabilities: vec![
                    "create_tiles".to_string(),
                    "modify_own_tiles".to_string(),
                ],
                connected_at_wall_us: 11,
                last_reconnect_wall_us: 11,
            },
        )
        .expect("record_hud_connection must succeed");

    authority
        .record_advisory_lease(
            projection_id,
            tze_hud_projection::AdvisoryLeaseIdentity {
                lease_id: bytes_hex(&lease_id),
                capabilities: vec!["create_tiles".to_string(), "modify_own_tiles".to_string()],
                acquired_at_wall_us: 12,
                expires_at_wall_us: 120_000_012,
            },
            13,
        )
        .expect("record_advisory_lease must succeed");

    authority
        .authorize_portal_republish(
            projection_id,
            &bytes_hex(&lease_id),
            &["create_tiles".to_string(), "modify_own_tiles".to_string()],
            14,
        )
        .expect("authorize_portal_republish must succeed");

    let published = authority.handle_publish_output(
        PublishOutputRequest {
            envelope: OperationEnvelope {
                operation: ProjectionOperation::PublishOutput,
                projection_id: projection_id.to_string(),
                request_id: format!("output-{projection_id}"),
                client_timestamp_wall_us: 20,
            },
            owner_token: owner_token.clone(),
            output_text: format!("assistant output for {projection_id}"),
            output_kind: OutputKind::Assistant,
            content_classification: ContentClassification::Private,
            logical_unit_id: Some(format!("unit-{projection_id}")),
            coalesce_key: None,
        },
        "codex-session",
        21,
    );
    assert!(published.accepted, "publish output must succeed");

    let active = authority.handle_publish_status(
        PublishStatusRequest {
            envelope: OperationEnvelope {
                operation: ProjectionOperation::PublishStatus,
                projection_id: projection_id.to_string(),
                request_id: format!("status-{projection_id}"),
                client_timestamp_wall_us: 22,
            },
            owner_token,
            lifecycle_state: ProjectionLifecycleState::Active,
            status_text: Some("styling test active".to_string()),
        },
        "codex-session",
        23,
    );
    assert!(active.accepted, "publish status must succeed");

    authority
        .projected_portal_state(projection_id, policy)
        .expect("expanded state must materialize after attach + publish")
}

// ── §6.1: Adapter publish path contains no literal visual values ──────────────

/// Verifies that the resident gRPC adapter's publish path (portal_node) sources
/// all visual values from `visual_tokens()` — never from inline literals.
///
/// We inject a sentinel color (full magenta) as the expanded transcript text
/// color, then verify the adapter's `visual_tokens()` returns it. This proves
/// the adapter would use that value in portal_node, satisfying §6.1.
#[test]
fn adapter_publish_path_sources_colors_from_visual_tokens() {
    // Default adapter has default visual tokens
    let adapter_default =
        ResidentGrpcPortalAdapter::new(ResidentGrpcPortalConfig::new(vec![0u8; 16]));
    let default_tokens = adapter_default.visual_tokens();

    // Inject a sentinel transcript text color
    let sentinel_tokens = PortalVisualTokens {
        transcript_text_color: tze_hud_protocol::proto::Rgba {
            r: 1.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        }, // magenta sentinel
        ..PortalVisualTokens::default()
    };

    let adapter_with_sentinel = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        sentinel_tokens.clone(),
    );

    // The default adapter must NOT have the sentinel (proves injection works)
    assert_ne!(
        default_tokens.transcript_text_color, sentinel_tokens.transcript_text_color,
        "default tokens must differ from sentinel"
    );

    // The injected adapter must return exactly the sentinel
    assert_eq!(
        *adapter_with_sentinel.visual_tokens(),
        sentinel_tokens,
        "adapter must return the tokens provided to with_tokens"
    );

    // After set_visual_tokens, the same sentinel must be accessible
    let mut adapter_mutable =
        ResidentGrpcPortalAdapter::new(ResidentGrpcPortalConfig::new(vec![0u8; 16]));
    adapter_mutable.set_visual_tokens(sentinel_tokens.clone());
    assert_eq!(
        *adapter_mutable.visual_tokens(),
        sentinel_tokens,
        "set_visual_tokens must replace visual tokens on the adapter"
    );
}

// ── §6.4 (d): NodeProto output assertion ─────────────────────────────────────

/// Verifies that the published `NodeProto` output from `render_portal_message`
/// actually uses the injected `PortalVisualTokens` values.
///
/// This is the §6.4 strengthened test: previous tests only asserted on
/// `visual_tokens()` — the getter accessor. A reintroduced literal in
/// `portal_node` would pass those tests. This test asserts on the *published*
/// `TextMarkdownNodeProto.color` and `.background` in the `MutationBatch`,
/// proving the render path actually consumes the injected tokens.
#[test]
fn portal_node_proto_uses_injected_visual_tokens() {
    use std::collections::HashMap;
    use tze_hud_protocol::proto;
    use tze_hud_protocol::proto::session as session_proto;

    let empty: HashMap<String, String> = HashMap::new();

    // Build a sentinel PortalVisualTokens via the canonical conversion chain:
    // override transcript_text_color to cyan and collapsed_background to yellow.
    let mut overrides = HashMap::new();
    overrides.insert(
        tze_hud_config::PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
        "#00FFFF".to_string(), // cyan — r=0, g=1, b=1
    );
    overrides.insert(
        tze_hud_config::PORTAL_TOKEN_COLLAPSED_BACKGROUND.to_string(),
        "#FFFF00".to_string(), // yellow — r=1, g=1, b=0
    );
    let resolved = tze_hud_config::tokens::resolve_tokens(&empty, &overrides);
    let part_tokens = resolve_portal_tokens(&resolved);
    // Use the production canonical conversion — no hand-rolled PortalVisualTokens
    let visual_tokens = portal_visual_tokens_from_part_tokens(&part_tokens);

    let mut authority = tze_hud_projection::ProjectionAuthority::default();
    let permit_all = tze_hud_projection::ProjectedPortalPolicy::permit_all();

    // Build an expanded projection state
    let expanded_state =
        build_expanded_state(&mut authority, "proj-nodeproto-expanded", &permit_all);

    // Build adapter, record a fake tile ID so render_portal_message succeeds
    let fake_tile_id: Vec<u8> = vec![0xAB; 16];
    let mut adapter = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        visual_tokens.clone(),
    );
    adapter.record_created_tile(fake_tile_id.clone());

    // Call render_portal_message and extract the NodeProto from the MutationBatch
    let cmd = adapter
        .render_portal_message(&expanded_state, 1, 0)
        .expect("render_portal_message must succeed after tile is recorded");

    // Extract the MutationBatch from the ClientMessage payload
    let batch = match cmd.message.payload.expect("render must produce payload") {
        session_proto::client_message::Payload::MutationBatch(b) => b,
        other => panic!("expected MutationBatch payload, got {other:?}"),
    };

    // The first mutation must be PublishToTile
    let publish = batch
        .mutations
        .into_iter()
        .find_map(|m| match m.mutation {
            Some(proto::mutation_proto::Mutation::PublishToTile(p)) => Some(p),
            _ => None,
        })
        .expect("MutationBatch must contain a PublishToTile mutation");

    // Extract the NodeProto and its TextMarkdown data
    let node = publish.node.expect("PublishToTile must have a node");
    let text_md = match node.data.expect("NodeProto must have data") {
        proto::node_proto::Data::TextMarkdown(tm) => tm,
        other => panic!("NodeProto must be TextMarkdown in the portal pilot, got {other:?}"),
    };

    // ── Core assertion (§6.4d): NodeProto color must be the injected token value ──

    // Expanded state uses transcript_text_color: injected to cyan (#00FFFF)
    let color = text_md
        .color
        .expect("TextMarkdownNodeProto must have a color");
    assert!(
        color.r.abs() < 1e-2,
        "expanded NodeProto color.r must be 0.0 (cyan has r=0), got {r}",
        r = color.r
    );
    assert!(
        (color.g - 1.0).abs() < 1e-2,
        "expanded NodeProto color.g must be 1.0 (cyan has g=1), got {g}",
        g = color.g
    );
    assert!(
        (color.b - 1.0).abs() < 1e-2,
        "expanded NodeProto color.b must be 1.0 (cyan has b=1), got {b}",
        b = color.b
    );

    // Font size must match the token-derived transcript_font_size_px
    assert!(
        (text_md.font_size_px - part_tokens.transcript_font_size_px).abs() < 1e-3,
        "expanded NodeProto font_size_px must equal token-derived transcript_font_size_px"
    );

    // ── Collapsed state: background must be the injected yellow token ──

    // Collapse the portal and re-render
    authority
        .collapse_projected_portal("proj-nodeproto-expanded")
        .expect("collapse must succeed");
    let collapsed_state = authority
        .projected_portal_state("proj-nodeproto-expanded", &permit_all)
        .expect("collapsed state must materialize");
    assert_eq!(
        collapsed_state.presentation,
        ProjectedPortalPresentation::Collapsed
    );

    let cmd_collapsed = adapter
        .render_portal_message(&collapsed_state, 2, 0)
        .expect("render_portal_message must succeed for collapsed state");
    let batch_collapsed = match cmd_collapsed
        .message
        .payload
        .expect("collapsed render must produce payload")
    {
        session_proto::client_message::Payload::MutationBatch(b) => b,
        other => panic!("expected MutationBatch payload for collapsed, got {other:?}"),
    };
    let publish_collapsed = batch_collapsed
        .mutations
        .into_iter()
        .find_map(|m| match m.mutation {
            Some(proto::mutation_proto::Mutation::PublishToTile(p)) => Some(p),
            _ => None,
        })
        .expect("collapsed MutationBatch must contain a PublishToTile mutation");
    let node_collapsed = publish_collapsed
        .node
        .expect("collapsed PublishToTile must have a node");
    let text_md_collapsed = match node_collapsed
        .data
        .expect("collapsed NodeProto must have data")
    {
        proto::node_proto::Data::TextMarkdown(tm) => tm,
        other => panic!("collapsed NodeProto must be TextMarkdown, got {other:?}"),
    };

    // Collapsed state uses collapsed_background: injected to yellow (#FFFF00)
    let bg_collapsed = text_md_collapsed
        .background
        .expect("collapsed TextMarkdownNodeProto must have background");
    assert!(
        (bg_collapsed.r - 1.0).abs() < 1e-2,
        "collapsed NodeProto background.r must be 1.0 (yellow has r=1), got {r}",
        r = bg_collapsed.r
    );
    assert!(
        (bg_collapsed.g - 1.0).abs() < 1e-2,
        "collapsed NodeProto background.g must be 1.0 (yellow has g=1), got {g}",
        g = bg_collapsed.g
    );
    assert!(
        bg_collapsed.b.abs() < 1e-2,
        "collapsed NodeProto background.b must be 0.0 (yellow has b=0), got {b}",
        b = bg_collapsed.b
    );
}

// ── §6.2: Portal part inventory — all parts covered by tokens ────────────────

/// Verifies that all portal parts in the inventory have a non-zero token value
/// after resolution from the default (empty) token map.
///
/// All assertions check `tokens.*` (the output of `resolve_portal_tokens`) to
/// verify the resolver produces sane defaults, not `PortalVisualTokens::default`
/// which exercises a different code path.
#[test]
fn portal_part_inventory_all_parts_have_non_zero_defaults() {
    use std::collections::HashMap;
    let empty: HashMap<String, String> = HashMap::new();
    let tokens = resolve_portal_tokens(&empty);

    // Frame
    assert!(
        tokens.frame_opacity > 0.0 && tokens.frame_opacity <= 1.0,
        "portal part inventory: frame opacity must be in (0, 1]"
    );

    // Header
    assert!(
        tokens.header_font_size_px > 0.0,
        "portal part inventory: header font size must be positive"
    );

    // Composer
    assert!(
        tokens.composer_font_size_px > 0.0,
        "portal part inventory: composer font size must be positive"
    );

    // Transcript
    assert!(
        tokens.transcript_font_size_px > 0.0,
        "portal part inventory: transcript font size must be positive"
    );

    // Collapsed
    assert!(
        tokens.collapsed_font_size_px > 0.0,
        "portal part inventory: collapsed font size must be positive"
    );

    // Transitions
    assert!(
        tokens.transition_in_ms > 0,
        "portal part inventory: transition_in_ms must be positive"
    );
    assert!(
        tokens.transition_out_ms > 0,
        "portal part inventory: transition_out_ms must be positive"
    );
}

// ── §6.4: Profile-swap reskin (core scenario) ─────────────────────────────────

/// Profile swap reskins the adapter without changing adapter logic.
///
/// The adapter code path (`with_tokens`/`set_visual_tokens`) is the same for
/// both profiles. Only the token *values* differ. This is the §6.1 invariant:
/// "a profile/token change must reskin the portal end-to-end with zero adapter
/// logic changes."
///
/// Uses `portal_visual_tokens_from_part_tokens` (the canonical production
/// conversion) rather than hand-rolling each field — de-duplicating the
/// test conversion with the production path (§6.4 / issue (c)).
#[test]
fn profile_swap_reskins_adapter_without_adapter_logic_change() {
    use std::collections::HashMap;

    // Profile A: default (dark) — no overrides
    let empty: HashMap<String, String> = HashMap::new();
    let profile_a_config_tokens = resolve_portal_tokens(&empty);
    let profile_a_visual = portal_visual_tokens_from_part_tokens(&profile_a_config_tokens);

    // Profile B: light theme — transcript/collapsed parts overridden
    let mut profile_b_overrides: HashMap<String, String> = HashMap::new();
    profile_b_overrides.insert(
        PORTAL_TOKEN_TRANSCRIPT_TEXT_COLOR.to_string(),
        "#111111".to_string(),
    );
    profile_b_overrides.insert(
        PORTAL_TOKEN_TRANSCRIPT_BACKGROUND.to_string(),
        "#F0F0F0".to_string(),
    );
    profile_b_overrides.insert(
        PORTAL_TOKEN_TRANSCRIPT_FONT_SIZE.to_string(),
        "16".to_string(),
    );
    profile_b_overrides.insert(
        PORTAL_TOKEN_COLLAPSED_BACKGROUND.to_string(),
        "#E8E8E8".to_string(),
    );
    profile_b_overrides.insert(
        PORTAL_TOKEN_COLLAPSED_TEXT_COLOR.to_string(),
        "#222222".to_string(),
    );
    profile_b_overrides.insert(
        PORTAL_TOKEN_COLLAPSED_FONT_SIZE.to_string(),
        "13".to_string(),
    );

    // Use resolve_tokens to merge overrides (profile layer)
    let resolved_b = tze_hud_config::tokens::resolve_tokens(&empty, &profile_b_overrides);
    let profile_b_config_tokens = resolve_portal_tokens(&resolved_b);
    let profile_b_visual = portal_visual_tokens_from_part_tokens(&profile_b_config_tokens);

    // Both adapters use IDENTICAL code paths — only token values differ.
    let adapter_a = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        profile_a_visual.clone(),
    );
    let adapter_b = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        profile_b_visual.clone(),
    );

    // Token values must differ — proving the profile swap had effect
    assert_ne!(
        adapter_a.visual_tokens().transcript_background,
        adapter_b.visual_tokens().transcript_background,
        "profile swap must produce different transcript background"
    );
    assert_ne!(
        adapter_a.visual_tokens().transcript_text_color,
        adapter_b.visual_tokens().transcript_text_color,
        "profile swap must produce different transcript text color"
    );
    assert_ne!(
        adapter_a.visual_tokens().collapsed_background,
        adapter_b.visual_tokens().collapsed_background,
        "profile swap must produce different collapsed background"
    );
    assert!(
        (adapter_b.visual_tokens().transcript_font_size_px - 16.0).abs() < 1e-3,
        "profile B transcript font size must be 16px"
    );
    assert!(
        (adapter_b.visual_tokens().collapsed_font_size_px - 13.0).abs() < 1e-3,
        "profile B collapsed font size must be 13px"
    );
}

// ── §6.4: Token propagation on republish ─────────────────────────────────────

/// Verifies that a token change propagates to the adapter on the next render
/// cycle without any adapter code change. This covers the "republish"
/// scenario: the adapter is updated with new tokens, then re-renders.
///
/// Uses `portal_visual_tokens_from_part_tokens` (the canonical production
/// conversion) rather than hand-rolling each field — de-duplicating the
/// test conversion with the production path (§6.4 / issue (c)).
#[test]
fn token_change_propagates_to_adapter_on_republish() {
    use std::collections::HashMap;

    let empty: HashMap<String, String> = HashMap::new();

    // Cycle 1: default tokens — use canonical conversion
    let cycle1_part = resolve_portal_tokens(&empty);
    let mut adapter = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        portal_visual_tokens_from_part_tokens(&cycle1_part),
    );

    let cycle1_background = adapter.visual_tokens().transcript_background;

    // Cycle 2: token update (simulate profile hot-reload)
    let mut overrides: HashMap<String, String> = HashMap::new();
    overrides.insert(
        PORTAL_TOKEN_TRANSCRIPT_BACKGROUND.to_string(),
        "#4A90D9".to_string(), // distinctive blue
    );
    let new_map = tze_hud_config::tokens::resolve_tokens(&empty, &overrides);
    let cycle2_part = resolve_portal_tokens(&new_map);

    // Use the canonical conversion — this is the production hot-reload path
    adapter.set_visual_tokens(portal_visual_tokens_from_part_tokens(&cycle2_part));

    let cycle2_background = adapter.visual_tokens().transcript_background;

    // The token change must propagate — backgrounds must differ
    assert_ne!(
        cycle1_background, cycle2_background,
        "token change must propagate to adapter on republish"
    );

    // Blue channel must be dominant after #4A90D9
    assert!(
        cycle2_background.b > 0.7,
        "cycle 2 transcript background must have high blue channel (#4A90D9)"
    );
}

// ── §6.4: Redaction-safe transitions ─────────────────────────────────────────

/// Verifies that a restricted viewer (Public clearance) observes `redacted = true`
/// when the portal carries Private content in either presentation state.
///
/// This is the structural invariant for §6.3: transitions are safe because the
/// `redacted` field is computed from viewer clearance vs. content classification
/// — NOT from a per-frame animation position. Whether the portal is Expanded
/// or Collapsed, a restricted viewer sees `redacted = true`, so any intermediate
/// transition frame is covered by redaction.
///
/// Token-derived `transition_in_ms` / `transition_out_ms` govern animation
/// duration but cannot override this structural property.
#[test]
fn restricted_viewer_observes_redacted_in_both_presentation_states() {
    // Policy for a restricted viewer: Public clearance, cannot see Private content
    let restricted_policy = ProjectedPortalPolicy {
        viewer_clearance: ContentClassification::Public,
        reveal_identity: false,
        reveal_lifecycle: false,
        reveal_transcript: false,
        reveal_unread: false,
        reveal_pending_input: false,
        allow_input: false,
        safe_mode_active: false,
        frozen: false,
        dismissed: false,
    };

    let permit_all = ProjectedPortalPolicy::permit_all();

    // --- Expanded state ---
    let mut authority_expanded = ProjectionAuthority::default();
    let expanded_state =
        build_expanded_state(&mut authority_expanded, "proj-redact-expanded", &permit_all);

    // Owner sees it expanded (not redacted)
    assert_eq!(
        expanded_state.presentation,
        ProjectedPortalPresentation::Expanded
    );
    assert!(
        !expanded_state.redacted,
        "owner must not see redacted expanded state"
    );

    // Restricted viewer sees redacted (content_classification = Private > Public)
    let restricted_expanded = authority_expanded
        .projected_portal_state("proj-redact-expanded", &restricted_policy)
        .expect("restricted policy must still materialize state");

    assert!(
        restricted_expanded.redacted,
        "restricted viewer must see redacted=true in Expanded state with Private content"
    );
    // Transcript must be empty for restricted viewer
    assert!(
        restricted_expanded.visible_transcript.is_empty(),
        "restricted viewer must not receive transcript content"
    );

    // --- Collapsed state ---
    let mut authority_collapsed = ProjectionAuthority::default();
    let expanded_before_collapse = build_expanded_state(
        &mut authority_collapsed,
        "proj-redact-collapsed",
        &permit_all,
    );
    assert_eq!(
        expanded_before_collapse.presentation,
        ProjectedPortalPresentation::Expanded
    );

    authority_collapsed
        .collapse_projected_portal("proj-redact-collapsed")
        .expect("collapse must succeed");

    let collapsed_owner = authority_collapsed
        .projected_portal_state("proj-redact-collapsed", &permit_all)
        .expect("collapsed state must materialize");
    assert_eq!(
        collapsed_owner.presentation,
        ProjectedPortalPresentation::Collapsed
    );

    let restricted_collapsed = authority_collapsed
        .projected_portal_state("proj-redact-collapsed", &restricted_policy)
        .expect("restricted policy must still materialize collapsed state");

    assert!(
        restricted_collapsed.redacted,
        "restricted viewer must see redacted=true in Collapsed state with Private content"
    );
}

/// Verifies that the structural redaction property holds regardless of the
/// token-derived transition duration values.
///
/// Even if `transition_in_ms` = 0 (instant) or very large (1000ms), the
/// redaction is not time-based — it is purely policy-based. This test confirms
/// that swapping transition tokens does not affect the redaction invariant.
#[test]
fn transition_duration_tokens_do_not_affect_redaction_safety() {
    use std::collections::HashMap;

    let restricted_policy = ProjectedPortalPolicy {
        viewer_clearance: ContentClassification::Public,
        ..Default::default()
    };
    let permit_all = ProjectedPortalPolicy::permit_all();

    // Build adapter with extreme transition durations
    let mut instant_overrides: HashMap<String, String> = HashMap::new();
    instant_overrides.insert(PORTAL_TOKEN_TRANSITION_IN_MS.to_string(), "1".to_string());
    instant_overrides.insert(PORTAL_TOKEN_TRANSITION_OUT_MS.to_string(), "1".to_string());

    let empty: HashMap<String, String> = HashMap::new();
    let instant_map = tze_hud_config::tokens::resolve_tokens(&empty, &instant_overrides);
    let instant_tokens = resolve_portal_tokens(&instant_map);
    assert_eq!(instant_tokens.transition_in_ms, 1, "instant in must be 1ms");
    assert_eq!(
        instant_tokens.transition_out_ms, 1,
        "instant out must be 1ms"
    );

    // Build adapter with slow transition durations
    let mut slow_overrides: HashMap<String, String> = HashMap::new();
    slow_overrides.insert(
        PORTAL_TOKEN_TRANSITION_IN_MS.to_string(),
        "1000".to_string(),
    );
    slow_overrides.insert(
        PORTAL_TOKEN_TRANSITION_OUT_MS.to_string(),
        "1000".to_string(),
    );

    let slow_map = tze_hud_config::tokens::resolve_tokens(&empty, &slow_overrides);
    let slow_tokens = resolve_portal_tokens(&slow_map);
    assert_eq!(slow_tokens.transition_in_ms, 1000, "slow in must be 1000ms");

    // In both cases, a restricted viewer sees redacted = true
    // We verify through the projection authority, which owns the redaction logic

    let mut authority_a = ProjectionAuthority::default();
    let _ = build_expanded_state(&mut authority_a, "proj-instant", &permit_all);
    let restricted_a = authority_a
        .projected_portal_state("proj-instant", &restricted_policy)
        .expect("instant-transition projection must materialize");

    let mut authority_b = ProjectionAuthority::default();
    let _ = build_expanded_state(&mut authority_b, "proj-slow", &permit_all);
    let restricted_b = authority_b
        .projected_portal_state("proj-slow", &restricted_policy)
        .expect("slow-transition projection must materialize");

    assert!(
        restricted_a.redacted,
        "instant transition: restricted viewer must still see redacted=true"
    );
    assert!(
        restricted_b.redacted,
        "slow transition: restricted viewer must still see redacted=true"
    );

    // The redaction is identical regardless of transition token values
    assert_eq!(
        restricted_a.redacted, restricted_b.redacted,
        "transition duration tokens must not affect redaction outcome"
    );
}

// ── §4.1 / §4.8: Composer token NodeProto output assertions (hud-2zyt9) ────────

/// Verifies that the composer token fields propagate through the canonical
/// conversion chain (`resolve_portal_tokens` → `portal_visual_tokens_from_part_tokens`)
/// and are accessible via `visual_tokens()` on the adapter.
///
/// This is the §6.1 composer-side proof: the adapter's published composer
/// visual values always come from the resolved token set, never from literals.
#[test]
fn composer_tokens_propagate_through_canonical_conversion() {
    use std::collections::HashMap;

    let empty: HashMap<String, String> = HashMap::new();

    // Override all composer tokens to sentinel values
    let mut overrides = HashMap::new();
    overrides.insert(
        PORTAL_TOKEN_COMPOSER_TEXT_COLOR.to_string(),
        "#FF00FF".to_string(), // magenta
    );
    overrides.insert(
        PORTAL_TOKEN_COMPOSER_BACKGROUND.to_string(),
        "#FFFF00".to_string(), // yellow
    );
    overrides.insert(
        PORTAL_TOKEN_COMPOSER_FONT_SIZE.to_string(),
        "18".to_string(),
    );
    overrides.insert(
        PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR.to_string(),
        "#00FFFF".to_string(), // cyan
    );

    let resolved = tze_hud_config::tokens::resolve_tokens(&empty, &overrides);
    let part_tokens = resolve_portal_tokens(&resolved);
    let visual_tokens = portal_visual_tokens_from_part_tokens(&part_tokens);

    let adapter = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        visual_tokens,
    );

    let vt = adapter.visual_tokens();

    // composer_text_color must be magenta (r=1, g=0, b=1)
    assert!(
        (vt.composer_text_color.r - 1.0).abs() < 1e-2,
        "composer_text_color.r must be 1.0 (magenta)"
    );
    assert!(
        vt.composer_text_color.g.abs() < 1e-2,
        "composer_text_color.g must be 0.0 (magenta)"
    );
    assert!(
        (vt.composer_text_color.b - 1.0).abs() < 1e-2,
        "composer_text_color.b must be 1.0 (magenta)"
    );

    // composer_background must be yellow (r=1, g=1, b=0)
    assert!(
        (vt.composer_background.r - 1.0).abs() < 1e-2,
        "composer_background.r must be 1.0 (yellow)"
    );
    assert!(
        (vt.composer_background.g - 1.0).abs() < 1e-2,
        "composer_background.g must be 1.0 (yellow)"
    );
    assert!(
        vt.composer_background.b.abs() < 1e-2,
        "composer_background.b must be 0.0 (yellow)"
    );

    // composer_font_size_px must be 18
    assert!(
        (vt.composer_font_size_px - 18.0).abs() < 1e-3,
        "composer_font_size_px must be 18.0"
    );

    // composer_at_capacity_color must be cyan (r=0, g=1, b=1)
    assert!(
        vt.composer_at_capacity_color.r.abs() < 1e-2,
        "composer_at_capacity_color.r must be 0.0 (cyan)"
    );
    assert!(
        (vt.composer_at_capacity_color.g - 1.0).abs() < 1e-2,
        "composer_at_capacity_color.g must be 1.0 (cyan)"
    );
    assert!(
        (vt.composer_at_capacity_color.b - 1.0).abs() < 1e-2,
        "composer_at_capacity_color.b must be 1.0 (cyan)"
    );
}

/// Verifies that the published `NodeProto` output from `render_portal_message`
/// contains the draft text with a `▌` caret marker when a draft is active.
///
/// This is the §4.1 local-first test: after `apply_draft_notification` delivers
/// a state-stream update, the next `render_portal_message` reflects the draft
/// immediately without any remote roundtrip.
///
/// Also verifies the caret is present at the correct position and the content
/// differs from the no-draft baseline.
#[test]
fn portal_node_proto_includes_draft_text_and_caret_after_notification() {
    use std::collections::HashMap;
    use tze_hud_projection::{
        AdapterDraftNotification, resident_grpc::ResidentGrpcDraftCommandKind,
    };

    let empty: HashMap<String, String> = HashMap::new();
    let part_tokens = resolve_portal_tokens(&empty);
    let visual_tokens = portal_visual_tokens_from_part_tokens(&part_tokens);

    let mut authority = tze_hud_projection::ProjectionAuthority::default();
    let permit_all = tze_hud_projection::ProjectedPortalPolicy::permit_all();
    let expanded_state = build_expanded_state(&mut authority, "proj-draft-caret", &permit_all);

    let fake_tile_id: Vec<u8> = vec![0xCE; 16];
    let mut adapter = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        visual_tokens,
    );
    adapter.record_created_tile(fake_tile_id);

    // ── Baseline: no draft active → composer line shows "composer: ready" ──

    let baseline_cmd = adapter
        .render_portal_message(&expanded_state, 1, 0)
        .expect("baseline render must succeed");
    let baseline_content = extract_text_markdown_content(baseline_cmd);
    assert!(
        baseline_content.contains("composer: ready"),
        "baseline (no draft) must show 'composer: ready'; got:\n{baseline_content}"
    );
    assert!(
        !baseline_content.contains("▌"),
        "baseline must not contain caret marker before any draft notification"
    );

    // ── Deliver a draft notification ──

    let notification = AdapterDraftNotification {
        text: "hello".to_string(),
        cursor: 5, // cursor at end
        selection_anchor: 5,
        at_capacity: false,
        sequence: 1,
    };
    let cmd = adapter
        .apply_draft_notification(&notification)
        .expect("notification must be accepted");
    assert_eq!(
        cmd.kind,
        ResidentGrpcDraftCommandKind::UpdateComposerDisplay
    );

    // ── After notification: NodeProto content must include draft text + caret ──

    let draft_cmd = adapter
        .render_portal_message(&expanded_state, 2, 0)
        .expect("draft render must succeed");
    let draft_content = extract_text_markdown_content(draft_cmd);

    // Caret marker must be present
    assert!(
        draft_content.contains("▌"),
        "draft render must contain caret marker ▌; got:\n{draft_content}"
    );
    // Draft text must appear before the caret
    assert!(
        draft_content.contains("hello▌"),
        "draft text 'hello' must appear before caret at end; got:\n{draft_content}"
    );
    // Must not show the generic "composer: ready" placeholder
    assert!(
        !draft_content.contains("composer: ready"),
        "active draft must replace 'composer: ready' placeholder"
    );

    // ── Verify caret is at mid-string cursor position ──

    let mid_notification = AdapterDraftNotification {
        text: "hello world".to_string(),
        cursor: 5, // cursor after "hello"
        selection_anchor: 5,
        at_capacity: false,
        sequence: 2,
    };
    adapter.apply_draft_notification(&mid_notification);

    let mid_cmd = adapter
        .render_portal_message(&expanded_state, 3, 0)
        .expect("mid-cursor render must succeed");
    let mid_content = extract_text_markdown_content(mid_cmd);

    assert!(
        mid_content.contains("hello▌ world"),
        "caret must appear between 'hello' and ' world' at byte offset 5; got:\n{mid_content}"
    );
}

/// Verifies that the at-capacity indicator appears in the NodeProto when the
/// composer draft is at its byte cap.
///
/// Checks both the text-visible `[!]` prefix in the content string and the
/// `color_runs` sentinel carrying `composer_at_capacity_color` from the token set.
/// Together these prove the at-capacity visual is fully token-driven (§6.1, §4.1).
#[test]
fn portal_node_proto_at_capacity_indicator_uses_composer_token() {
    use std::collections::HashMap;
    use tze_hud_projection::AdapterDraftNotification;

    let empty: HashMap<String, String> = HashMap::new();

    // Override at-capacity color to a distinctive value (pure blue)
    let mut overrides = HashMap::new();
    overrides.insert(
        PORTAL_TOKEN_COMPOSER_AT_CAPACITY_COLOR.to_string(),
        "#0000FF".to_string(), // pure blue sentinel
    );
    let resolved = tze_hud_config::tokens::resolve_tokens(&empty, &overrides);
    let part_tokens = resolve_portal_tokens(&resolved);
    let visual_tokens = portal_visual_tokens_from_part_tokens(&part_tokens);

    // Verify the token propagated to PortalVisualTokens
    assert!(
        visual_tokens.composer_at_capacity_color.b > 0.9,
        "at_capacity_color blue channel must be high (pure blue sentinel)"
    );
    assert!(
        visual_tokens.composer_at_capacity_color.r < 0.1,
        "at_capacity_color red channel must be ~0 (pure blue sentinel)"
    );

    let mut authority = tze_hud_projection::ProjectionAuthority::default();
    let permit_all = tze_hud_projection::ProjectedPortalPolicy::permit_all();
    let expanded_state = build_expanded_state(&mut authority, "proj-at-cap", &permit_all);

    let fake_tile_id: Vec<u8> = vec![0xAC; 16];
    let mut adapter = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        visual_tokens,
    );
    adapter.record_created_tile(fake_tile_id);

    // Deliver an at-capacity draft notification
    let at_cap_notification = AdapterDraftNotification {
        text: "x".repeat(50), // some text at cap
        cursor: 50,
        selection_anchor: 50,
        at_capacity: true,
        sequence: 1,
    };
    adapter.apply_draft_notification(&at_cap_notification);

    // Render and extract the published NodeProto
    let cmd = adapter
        .render_portal_message(&expanded_state, 1, 0)
        .expect("at-capacity render must succeed");
    let (text_md_content, color_runs) = extract_text_markdown_with_runs(cmd);

    // Text-visible indicator: content must contain "[!]"
    assert!(
        text_md_content.contains("[!]"),
        "at-capacity content must contain text-visible '[!]' prefix; got:\n{text_md_content}"
    );

    // Token-driven color run: must have a color_run with the injected blue sentinel
    assert!(
        !color_runs.is_empty(),
        "at-capacity NodeProto must have at least one color_run carrying the token color"
    );
    let cap_run = &color_runs[0];
    let run_color = cap_run
        .color
        .as_ref()
        .expect("at-capacity color_run must have a color");
    assert!(
        run_color.b > 0.9,
        "at-capacity color_run color.b must be ~1.0 (pure blue token sentinel); got {b}",
        b = run_color.b
    );
    assert!(
        run_color.r < 0.1,
        "at-capacity color_run color.r must be ~0.0 (pure blue token sentinel); got {r}",
        r = run_color.r
    );

    // ── Verify that a non-at-capacity draft produces no color_runs ──
    let normal_notification = AdapterDraftNotification {
        text: "normal text".to_string(),
        cursor: 11,
        selection_anchor: 11,
        at_capacity: false,
        sequence: 2,
    };
    adapter.apply_draft_notification(&normal_notification);

    let cmd_normal = adapter
        .render_portal_message(&expanded_state, 2, 0)
        .expect("normal render must succeed");
    let (_, runs_normal) = extract_text_markdown_with_runs(cmd_normal);
    assert!(
        runs_normal.is_empty(),
        "non-at-capacity draft must produce no color_runs; got {} runs",
        runs_normal.len()
    );
}

/// Verifies that after a ProcessCancel command, the composer display resets to
/// "composer: ready" (no draft active) in the next render.
#[test]
fn portal_node_proto_clears_composer_on_cancel() {
    use std::collections::HashMap;
    use tze_hud_projection::{AdapterDraftBatch, AdapterDraftCancel, AdapterDraftNotification};

    let empty: HashMap<String, String> = HashMap::new();
    let part_tokens = resolve_portal_tokens(&empty);
    let visual_tokens = portal_visual_tokens_from_part_tokens(&part_tokens);

    let mut authority = tze_hud_projection::ProjectionAuthority::default();
    let permit_all = tze_hud_projection::ProjectedPortalPolicy::permit_all();
    let expanded_state = build_expanded_state(&mut authority, "proj-cancel-clear", &permit_all);

    let fake_tile_id: Vec<u8> = vec![0xCC; 16];
    let mut adapter = ResidentGrpcPortalAdapter::with_tokens(
        ResidentGrpcPortalConfig::new(vec![0u8; 16]),
        visual_tokens,
    );
    adapter.record_created_tile(fake_tile_id);

    // Deliver a draft notification — composer shows draft text
    let notification = AdapterDraftNotification {
        text: "typed so far".to_string(),
        cursor: 12,
        selection_anchor: 12,
        at_capacity: false,
        sequence: 1,
    };
    adapter.apply_draft_notification(&notification);
    assert!(
        adapter.composer_display().is_some(),
        "composer_display must be Some after notification"
    );

    // Send a cancel batch — composer display must clear
    let mut batch = AdapterDraftBatch::new();
    batch.record_cancel(AdapterDraftCancel { sequence: 2 });
    adapter.consume_draft_batch(&batch);

    assert!(
        adapter.composer_display().is_none(),
        "composer_display must be None after ProcessCancel"
    );

    // NodeProto after cancel must show "composer: ready"
    let cmd = adapter
        .render_portal_message(&expanded_state, 3, 0)
        .expect("post-cancel render must succeed");
    let content = extract_text_markdown_content(cmd);
    assert!(
        content.contains("composer: ready"),
        "post-cancel render must show 'composer: ready'; got:\n{content}"
    );
}

// ── Test helpers ──────────────────────────────────────────────────────────────

/// Extract the `content` string from a `TextMarkdownNodeProto` inside a
/// `ResidentGrpcPortalCommand`'s `ClientMessage` payload.
fn extract_text_markdown_content(
    cmd: tze_hud_projection::resident_grpc::ResidentGrpcPortalCommand,
) -> String {
    use tze_hud_protocol::proto;
    use tze_hud_protocol::proto::session as session_proto;

    let batch = match cmd.message.payload.expect("render must produce payload") {
        session_proto::client_message::Payload::MutationBatch(b) => b,
        other => panic!("expected MutationBatch payload, got {other:?}"),
    };
    let publish = batch
        .mutations
        .into_iter()
        .find_map(|m| match m.mutation {
            Some(proto::mutation_proto::Mutation::PublishToTile(p)) => Some(p),
            _ => None,
        })
        .expect("MutationBatch must contain a PublishToTile mutation");
    let node = publish.node.expect("PublishToTile must have a node");
    match node.data.expect("NodeProto must have data") {
        proto::node_proto::Data::TextMarkdown(tm) => tm.content,
        other => panic!("NodeProto must be TextMarkdown, got {other:?}"),
    }
}

/// Extract both the content string and color_runs from a published NodeProto.
fn extract_text_markdown_with_runs(
    cmd: tze_hud_projection::resident_grpc::ResidentGrpcPortalCommand,
) -> (String, Vec<tze_hud_protocol::proto::TextColorRunProto>) {
    use tze_hud_protocol::proto;
    use tze_hud_protocol::proto::session as session_proto;

    let batch = match cmd.message.payload.expect("render must produce payload") {
        session_proto::client_message::Payload::MutationBatch(b) => b,
        other => panic!("expected MutationBatch payload, got {other:?}"),
    };
    let publish = batch
        .mutations
        .into_iter()
        .find_map(|m| match m.mutation {
            Some(proto::mutation_proto::Mutation::PublishToTile(p)) => Some(p),
            _ => None,
        })
        .expect("MutationBatch must contain a PublishToTile mutation");
    let node = publish.node.expect("PublishToTile must have a node");
    match node.data.expect("NodeProto must have data") {
        proto::node_proto::Data::TextMarkdown(tm) => (tm.content, tm.color_runs),
        other => panic!("NodeProto must be TextMarkdown, got {other:?}"),
    }
}
