## ADDED Requirements

### Requirement: Portal Disconnect Presentation

When the stream or session driving a text stream portal drops mid-stream, the portal SHALL retain its last coherent transcript window and SHALL present a visible degraded treatment rather than blanking, freezing silently as if live, or fabricating continued liveness. The retained window MUST preserve every already-committed logical transcript unit per the existing Coherent Transcript Coalescing requirement; a disconnect MUST NOT collapse the retained window or drop committed units. The degraded treatment — dimming, stale marker, and a disconnect affordance — SHALL resolve entirely from design tokens through the existing component-profile path and MUST NOT use hardcoded compositor styling. Live-only activity signals (typing/activity indicators, ephemeral-realtime hover or interim state) SHALL clear on disconnect so the surface does not imply an active stream. The disconnect affordance and any stale marker SHALL convey only connection geometry/state and MUST remain present and content-free under redaction, exactly like the existing scroll-position indicator: a viewer not permitted to see the transcript still sees that the portal is disconnected, but no transcript content is revealed by the disconnect treatment. The disconnect presentation MUST NOT itself escalate interruption class; a portal going stale is ambient state, not a notification, consistent with the Ambient Portal Attention Defaults requirement.

Source: RFC 0013 §3.2 and §4.4, `about/heart-and-soul/vision.md` (visual identity is modular), CLAUDE.md anti-pattern "treating graceful degradation as a bug", `crates/tze_hud_projection/src/contract.rs` (ProjectionLifecycleState::Degraded, HudUnavailable), `crates/tze_hud_projection/src/authority.rs` (mark_hud_disconnected)
Scope: v1-mandatory

#### Scenario: mid-stream drop retains last coherent window

- **WHEN** the driving stream or session drops while a portal has a non-empty retained transcript window
- **THEN** the portal SHALL continue to display the last coherent transcript window
- **AND** no already-committed logical transcript unit within that window SHALL be dropped because of the disconnect

#### Scenario: stale treatment is token-resolved, not hardcoded

- **WHEN** a portal enters the disconnected state and renders its degraded treatment
- **THEN** the dimming, stale marker, and disconnect affordance SHALL resolve from the active design tokens
- **AND** no color, opacity, typography, or stroke value of the degraded treatment SHALL come from a hardcoded compositor value

#### Scenario: liveness signals clear on disconnect

- **WHEN** a portal that was showing a typing or activity indicator loses its driving stream
- **THEN** the typing and activity indicators SHALL clear
- **AND** the surface SHALL NOT present any signal implying the stream is still active

#### Scenario: disconnect indicator is geometry-only under redaction

- **WHEN** the driving stream drops for a portal whose transcript the current viewer is not permitted to see
- **THEN** the disconnect indicator SHALL remain present and reflect the disconnected state
- **AND** it SHALL NOT reveal transcript content or identity beyond the existing neutral redaction treatment

#### Scenario: going stale does not self-escalate attention

- **WHEN** a portal transitions to the disconnected/stale state
- **THEN** the portal's attention presentation SHALL remain ambient or gentle
- **AND** the disconnect SHALL NOT be raised to a stronger interruption class merely because the stream dropped

### Requirement: Portal Stale-Content Degradation Contract

Text stream portals SHALL define when displayed content is considered live versus stale, bounded by the existing lease orphan/grace lifecycle rather than a second independent timer authority. After a bounded liveness gap on the driving stream — no committed transcript progress and no heartbeat/liveness signal within the configured degraded threshold — the portal's connection SHALL be treated as degraded and its displayed content SHALL be marked stale. The degraded window SHALL be bounded by the lease grace already defined for the orphan path: when the lease grace expires, the governed surface is removed under the existing lease rules and the stale content is no longer displayed. Entering and presenting the degraded/stale state is runtime-owned presentation timing: arrival timestamps remain advisory and the runtime decides when to render the degraded transition, consistent with the arrival-time-versus-presentation-time contract. Liveness, disconnect, and degraded-threshold metadata that the surface consumes MUST follow the existing typed clock-domain convention (`_wall_us` for wall-clock, `_mono_us` for monotonic) and MUST NOT introduce a presentation-authoritative arrival deadline.

Source: RFC 0013 §4.4, RFC 0008 (lease grace/orphan lifecycle), `about/craft-and-care/engineering-bar.md` §2, CLAUDE.md core rule "arrival time ≠ presentation time", `crates/tze_hud_projection/src/authority.rs` (last_disconnect_wall_us, mark_hud_disconnected)
Scope: v1-mandatory

#### Scenario: content goes stale after the degraded threshold

- **WHEN** a portal's driving stream produces no committed progress and no liveness signal for longer than the configured degraded threshold
- **THEN** the portal's connection SHALL be treated as degraded
- **AND** the displayed content SHALL be marked stale under the disconnect presentation treatment

#### Scenario: staleness is bounded by lease grace

- **WHEN** a portal remains disconnected until its lease grace expires
- **THEN** the governed surface SHALL be removed under the existing lease orphan rules
- **AND** the stale content SHALL no longer be displayed after grace expiry

#### Scenario: degraded transition timing is runtime-owned

- **WHEN** the runtime detects the liveness gap that qualifies a portal as degraded
- **THEN** the runtime SHALL decide when to present the degraded transition rather than treating any arrival timestamp as a presentation deadline
- **AND** the degraded treatment SHALL still appear within the bounded liveness/grace window

#### Scenario: degradation metadata uses typed clock domains

- **WHEN** the surface consumes disconnect, heartbeat, or degraded-threshold timing metadata
- **THEN** wall-clock fields SHALL use `_wall_us` and monotonic fields SHALL use `_mono_us`
- **AND** none of these fields SHALL override runtime presentation control

### Requirement: Portal Reconnect and Resume Presentation

When a portal's driving session re-attaches before lease grace expiry, the portal SHALL resume from the retained coherent visible transcript window the projection authority preserved, clear the degraded/stale treatment, and restore live presentation without losing already-committed transcript units. Resume SHALL preserve `logical_unit_id` continuity: a logical transcript unit that was in progress at disconnect and is continued on resume SHALL update in place rather than being duplicated as a new unit, and resumed appends SHALL coalesce under the existing state-stream Coherent Transcript Coalescing and Sustained Streaming Cadence rules. Resume SHALL materialize only the bounded retained visible window into scene nodes per the Bounded Transcript Viewport requirement; it MUST NOT reconstruct full transcript history into the scene graph. Pending HUD input and acknowledgement state restored on resume SHALL follow the existing input-inbox contract and MUST NOT be silently dropped by the reconnect. After lease grace expiry (session death), the surface is gone: a subsequent attach SHALL start a fresh portal under a new lease rather than silently reviving the removed surface or presenting pre-death stale content as live. Resume MUST respect the current viewer's redaction policy at every frame of the transition: a restricted viewer never sees transcript content flash during the stale-to-live transition.

Source: RFC 0013 §3.3 and §4.4, `openspec/specs/cooperative-hud-projection/spec.md` (External Projection State Authority — reconnect preserves projection state), `openspec/specs/external-agent-projection-authority/spec.md` (Multi-Session Lifecycle Management — reconnect bookkeeping), `crates/tze_hud_projection/src/authority.rs` (ReconnectBookkeeping, reconnect_count, last_reconnect_wall_us), `.claude/skills/hud-projection/SKILL.md` (detach/re-attach)
Scope: v1-mandatory

#### Scenario: reconnect before grace resumes from retained window

- **WHEN** a portal's driving session re-attaches before lease grace expiry
- **THEN** the portal SHALL resume from the retained coherent visible transcript window
- **AND** the degraded/stale treatment SHALL clear and live presentation SHALL resume
- **AND** no already-committed transcript unit from the retained window SHALL be lost

#### Scenario: continued logical unit updates in place

- **WHEN** a logical transcript unit that was in progress at disconnect is continued after reconnect with the same `logical_unit_id`
- **THEN** the portal SHALL update that unit in place
- **AND** it SHALL NOT render the continuation as a duplicate transcript unit

#### Scenario: resume materializes only the bounded window

- **WHEN** a portal resumes a session whose retained transcript history exceeds the visible viewport
- **THEN** only the bounded retained visible or immediately scrollable window SHALL be materialized into scene nodes
- **AND** the full retained history SHALL NOT be reconstructed into the scene graph

#### Scenario: resume preserves pending input

- **WHEN** the portal resumes a session that had non-terminal pending HUD input at disconnect
- **THEN** the preserved pending input and acknowledgement state SHALL remain available through the existing input-inbox contract
- **AND** the reconnect SHALL NOT silently drop those non-terminal items

#### Scenario: attach after grace starts a fresh portal

- **WHEN** a session attaches after the prior portal's lease grace already expired
- **THEN** the runtime SHALL start a fresh portal under a new lease
- **AND** it SHALL NOT silently revive the removed surface or present pre-death stale content as live

#### Scenario: stale-to-live transition respects redaction

- **WHEN** a portal resumes for a viewer whose policy redacts its transcript
- **THEN** every frame of the stale-to-live transition SHALL show the neutral redaction treatment in place of transcript content
- **AND** no transcript content SHALL flash during the transition

## MODIFIED Requirements

### Requirement: Governance, Privacy, and Override Compliance

Text stream portals SHALL obey the same lease, privacy, redaction, dismissal, freeze, and safe-mode rules as any other governed surface. Portal identity, transcript content, and activity metadata MUST NOT bypass viewer-class filtering or shell overrides because they are text. Collapsed portal cards MUST NOT be treated as automatically safe metadata. When the owning session disconnects, the lease orphan lifecycle and the viewer-facing disconnect/resume presentation SHALL stay coherent: the visible disconnect, stale, and resume treatments defined by the Portal Disconnect Presentation, Portal Stale-Content Degradation Contract, and Portal Reconnect and Resume Presentation requirements operate within the bounds of the lease orphan/grace lifecycle, and grace expiry removes the surface under the existing lease rules.

#### Scenario: portal content redacts under viewer policy

- **WHEN** portal content exceeds the current viewer's permitted classification
- **THEN** the portal SHALL be redacted under the runtime's existing privacy policy rather than exposing the transcript content

#### Scenario: portal suspends under safe mode

- **WHEN** the runtime enters safe mode while a portal is active
- **THEN** portal updates SHALL suspend under the same shell and lease rules as other content-layer surfaces

#### Scenario: collapsed portal preserves geometry while redacted

- **WHEN** the current viewer is not permitted to see a portal's identity or transcript state
- **THEN** the portal SHALL preserve its geometry, suppress transcript previews and activity details, and replace visible content with the runtime's neutral redaction treatment

#### Scenario: disconnected portal follows orphan path

- **WHEN** the owning resident portal session disconnects unexpectedly
- **THEN** the lease SHALL transition through the normal orphan lifecycle, the visible portal SHALL present the degraded/stale treatment over its last coherent state per the Portal Disconnect Presentation requirement, and grace expiry SHALL remove the governed surface under the existing lease rules
- **AND** a re-attach before grace expiry SHALL resume per the Portal Reconnect and Resume Presentation requirement rather than starting a fresh portal

#### Scenario: freeze does not disclose viewer intent

- **WHEN** a portal is active while the runtime freezes visible scene mutation
- **THEN** adapters SHALL observe only the existing generic queue-pressure or dropped-mutation semantics rather than a portal-specific freeze signal
