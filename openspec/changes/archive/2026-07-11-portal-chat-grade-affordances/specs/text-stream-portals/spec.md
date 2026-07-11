# text-stream-portals Delta: chat-grade transcript affordances

## ADDED Requirements

### Requirement: Viewer Turn Delivery Acknowledgement

The portal SHALL present an ambient per-turn delivery cue on the viewer's echoed turn reflecting the runtime's already-tracked input delivery state, so the viewer can see whether their reply reached the owning adapter without asking. The cue SHALL distinguish at least three presentation classes: in-flight (Pending or Deferred), delivered (Delivered or Handled), and failed (Rejected or Expired). The cue is local presentation of state the runtime already owns: rendering it MUST NOT introduce a new adapter round trip, and the viewer's read/seen state MUST NOT be disclosed back to the adapter as a side effect of rendering. The cue SHALL resolve its visual treatment from design tokens via the portal's component profile, SHALL remain subordinate to the Ambient Portal Attention Defaults requirement (a delivery transition is not an attention event), and a failed cue SHALL stay on the affected turn rather than escalating interruption class. Delivery cues are portal-surface presentation and SHALL redact together with the transcript under the Governance, Privacy, and Override Compliance requirement.

Source: §Viewer Reply Echo, `crates/tze_hud_projection/src/contract.rs` (InputDeliveryState: Pending, Delivered, Deferred, Handled, Rejected, Expired), `crates/tze_hud_projection/src/authority.rs` (delivery state tracked into ProjectedPortalState)
Scope: promotion (rendering under hud-g1ena)

#### Scenario: pending then delivered tick

- **WHEN** a viewer submits an accepted reply and the adapter subsequently acknowledges taking delivery of it
- **THEN** the echoed viewer turn SHALL first present the in-flight cue and then transition to the delivered cue
- **AND** neither transition SHALL raise the portal's interruption class or unread count

#### Scenario: failed delivery is visible on the turn

- **WHEN** a submitted reply's delivery state becomes Rejected or Expired after it was echoed
- **THEN** the echoed viewer turn SHALL present the failed cue in place of the delivered cue
- **AND** the failure SHALL be presented ambiently on the turn itself rather than as a separate notification

#### Scenario: delivery cue requires no adapter round trip

- **WHEN** the runtime renders or updates a delivery cue
- **THEN** the cue SHALL be driven entirely from runtime-owned delivery state
- **AND** no additional adapter request SHALL be issued to render it
- **AND** no viewer read/seen signal SHALL be sent to the adapter as a side effect

### Requirement: Unread Divider and Ambient Unread Count

The portal SHALL render its already-tracked unread output count as (a) an in-transcript unread divider marking the boundary between seen and unseen transcript units when the viewer returns to a portal with unread content in the retained window, and (b) a compact ambient unread count in the portal's collapsed or peripheral presentation. Both presentations SHALL clear locally when the viewer views the tail (local-first; no adapter round trip to clear). The unread presentation MUST stay within the Ambient Portal Attention Defaults: a growing count SHALL update the rendered number or divider position but MUST NOT self-escalate interruption class, animate repeatedly, or behave like a notification. The divider SHALL count only agent-authored transcript units — echoed viewer turns are never unread per the Viewer Reply Echo requirement. When unread transcript units have been coalesced or have scrolled out of the bounded retained window, the count MAY exceed the units visibly below the divider, and the divider SHALL sit at the oldest retained unseen unit.

Source: §Ambient Portal Attention Defaults, §Viewer Reply Echo, §Bounded Transcript Viewport, `crates/tze_hud_projection/src/contract.rs` (unread_output_count plumbed but unrendered)
Scope: promotion (rendering under hud-g1ena)

#### Scenario: divider marks the unread boundary

- **WHEN** a viewer focuses a portal whose retained window contains transcript units appended since the viewer last viewed the tail
- **THEN** an unread divider SHALL render at the boundary before the oldest retained unseen unit
- **AND** the divider and count SHALL clear locally once the viewer views the tail

#### Scenario: unread count stays ambient under backlog growth

- **WHEN** the unread count grows while the viewer is away or scrolled up
- **THEN** the rendered count SHALL update in place with ambient, token-styled treatment
- **AND** the portal SHALL NOT escalate interruption class, flash, or re-animate per increment

#### Scenario: viewer echo never counts as unread

- **WHEN** a viewer's own reply is echoed while the viewer is scrolled away from the tail
- **THEN** the unread divider and count SHALL NOT include the echoed viewer turn

### Requirement: Jump-to-Latest Affordance

When the viewer's scroll position leaves the transcript tail, the portal SHALL present a local-first jump-to-latest affordance that, when activated, returns the viewport to the tail and resumes tail-follow. While the viewer is scrolled away, incoming appends MUST NOT move the viewer's viewport (scroll position remains authoritative per the Transcript Interaction Contract); tail-follow resumes only through the affordance or the viewer scrolling back to the tail themselves. The affordance MAY carry the ambient unread count. Activation SHALL acknowledge locally under the existing interaction latency contract, and an adapter MUST NOT be able to trigger the jump on the viewer's behalf.

Source: §Transcript Interaction Contract, §Ambient Portal Attention Defaults
Scope: promotion (rendering under hud-g1ena)

#### Scenario: affordance appears when scrolled away and returns to tail

- **WHEN** the viewer scrolls the transcript away from the tail
- **THEN** a jump-to-latest affordance SHALL become visible with ambient treatment
- **AND** activating it SHALL return the viewport to the tail locally and resume tail-follow

#### Scenario: appends do not yank the viewport while scrolled away

- **WHEN** new transcript units arrive while the viewer is scrolled away from the tail
- **THEN** the viewer's scroll position SHALL NOT change
- **AND** the affordance (and any unread count it carries) SHALL update ambiently instead

#### Scenario: adapter cannot force the jump

- **WHEN** an adapter attempts to reposition the viewport to the tail while the viewer is scrolled away
- **THEN** the runtime SHALL disregard the repositioning per the Transcript Interaction Contract
- **AND** only viewer action SHALL resume tail-follow

### Requirement: Ambient Per-Turn Timestamps

The portal SHALL be able to present per-turn arrival times sourced from the transcript unit's typed wall-clock arrival metadata (`appended_at_wall_us`, wall-clock domain). Timestamp presentation SHALL be ambient and subordinate: token-styled, visually secondary to turn content, and never the source of attention escalation. Timestamps are presentation of arrival metadata the runtime already retains; rendering them MUST NOT require adapter cooperation, and adapter-supplied content MUST NOT be able to forge the runtime-assigned arrival time. Clock-domain typing SHALL be preserved end to end: wall-clock arrival time SHALL NOT be conflated with media/display-clock presentation timing. Timestamp visibility and granularity (per-turn, grouped, or on-demand) are governed by the portal's component profile and design tokens rather than mandated at the pixel level.

Source: §Transport-Agnostic Stream Boundary (typed clock domains), `crates/tze_hud_projection/src/contract.rs` (appended_at_wall_us)
Scope: promotion (rendering under hud-g1ena)

#### Scenario: turns present runtime-assigned arrival time

- **WHEN** the portal presents timestamps for retained transcript units
- **THEN** each presented time SHALL derive from the runtime-assigned wall-clock arrival metadata of that unit
- **AND** adapter-supplied content SHALL NOT override the runtime-assigned arrival time

#### Scenario: timestamps stay visually subordinate

- **WHEN** timestamps are visible in the transcript
- **THEN** their treatment SHALL resolve from design tokens as secondary presentation
- **AND** timestamp changes or boundaries SHALL NOT raise the portal's interruption class

### Requirement: Agent Activity and Streaming Cue

While the owning adapter is actively appending to the transcript, the portal SHALL be able to present an ambient activity cue: a streaming cursor or equivalent live-writing treatment at the transcript tail, and optionally a compact typing-style indicator in the portal's header or collapsed presentation. The cue SHALL derive from observed append activity or the existing activity metadata — it MUST NOT require a new adapter-side "typing" protocol message. The cue SHALL remain strictly subordinate to the Ambient Portal Attention Defaults requirement: continuous streaming SHALL NOT re-trigger attention, and the cue SHALL quiesce promptly when appends stop. The activity cue is activity metadata under the Governance, Privacy, and Override Compliance requirement: it SHALL suppress together with transcript previews when the portal is redacted for the current viewer and SHALL freeze under safe-mode and freeze rules like other portal presentation.

Source: §Ambient Portal Attention Defaults (typing indicator remains ambient), §Governance, Privacy, and Override Compliance (collapsed portal preserves geometry while redacted)
Scope: promotion (rendering under hud-g1ena)

#### Scenario: streaming cursor while agent writes

- **WHEN** transcript appends are actively streaming into the portal
- **THEN** the tail MAY present a streaming cursor or live-writing treatment with ambient token-styled presentation
- **AND** the cue SHALL quiesce promptly once appends stop

#### Scenario: activity cue is not a notification

- **WHEN** an adapter streams appends continuously over an extended period
- **THEN** the activity cue SHALL NOT re-escalate or repeat attention behavior
- **AND** the portal's interruption class SHALL remain at its policy-assigned level

#### Scenario: activity cue suppressed under redaction

- **WHEN** the current viewer's policy redacts the portal
- **THEN** the activity cue SHALL be suppressed along with transcript previews and activity details

### Requirement: First-Run Empty Portal Treatment

A portal surface whose retained transcript window contains no transcript units SHALL present a friendly, token-styled empty-state treatment identifying the portal and inviting interaction (for example, identity plus a short ready line), rather than a literal placeholder string such as `<empty projection stream>`. The empty-state treatment SHALL resolve from design tokens via the portal's component profile, SHALL respect redaction (identity and inviting copy suppress under the collapsed-redaction rules), and SHALL yield immediately to real content on the first transcript unit. When the portal is attached but not yet connected, the Connecting State Distinction requirement takes precedence over the plain empty treatment.

Source: `crates/tze_hud_projection/src/resident_grpc.rs` (`<empty projection stream>` literal), §Governance, Privacy, and Override Compliance
Scope: promotion (rendering under hud-g1ena)

#### Scenario: empty portal renders a designed empty state

- **WHEN** a connected portal has an empty retained transcript window
- **THEN** the surface SHALL present the token-styled empty-state treatment instead of a literal placeholder string
- **AND** the first appended transcript unit SHALL replace the empty state immediately

#### Scenario: empty state redacts like identity metadata

- **WHEN** the current viewer is not permitted the portal's identity
- **THEN** the empty-state treatment SHALL suppress identity and inviting copy under the existing redaction treatment

### Requirement: Connecting State Distinction

The portal SHALL distinguish three connection presentations: connecting (attached but the owning session has not yet established its first connection), connected (live), and degraded/disconnected (previously connected, now dropped). The connecting treatment SHALL be visually distinct from the degraded treatment so a portal that is starting up does not read as failing. Transition into and out of the connecting presentation SHALL be ambient (not an attention event). The degraded/disconnected path — freeze at last coherent state, orphan lifecycle, grace expiry — remains governed by the existing Governance, Privacy, and Override Compliance requirement and is unchanged by this requirement.

Source: §Governance, Privacy, and Override Compliance (disconnected portal follows orphan path), `crates/tze_hud_projection/src/contract.rs` (connection_degraded), epic hud-3jxfr (no-connecting-state-distinct-from-disconnected)
Scope: promotion (rendering under hud-g1ena)

#### Scenario: never-connected portal shows connecting, not degraded

- **WHEN** a portal is attached and its owning session has not yet established its first connection
- **THEN** the portal SHALL present the connecting treatment
- **AND** the degraded/disconnected treatment SHALL NOT be used for the never-connected case

#### Scenario: first connection transitions ambiently

- **WHEN** the owning session establishes its first connection
- **THEN** the portal SHALL transition from connecting to connected presentation without raising interruption class

#### Scenario: drop after connection uses degraded treatment

- **WHEN** a previously connected portal's session drops
- **THEN** the portal SHALL present the existing degraded/disconnected treatment and follow the existing orphan lifecycle

## MODIFIED Requirements

### Requirement: Viewer Reply Echo

A text stream portal SHALL maintain two distinct, separately-bounded histories: an INPUT history of the viewer's own accepted submissions, and an OUTPUT transcript of agent-authored content only. The two histories are separate streams and SHALL NOT be materialized as a single combined transcript unit sequence.

When a viewer submits a reply through a text stream portal composer and the submission is accepted, the runtime SHALL echo the submitted text into the portal's INPUT history as a viewer-authored turn, so the two-way conversation is visible on the surface rather than the viewer's own words disappearing into the adapter inbox. The INPUT history SHALL be presented in the portal's input region beneath a top-anchored composer, with successive viewer turns stacked and separated by token-styled turn dividers (the viewer-echo stack); the runtime SHALL retain only a bounded, newest-fit window of the INPUT history, obeying the Bounded Transcript Viewport rules for its own region rather than mirroring unbounded input history into scene nodes. An accepted viewer submission SHALL NOT be appended to the OUTPUT/agent transcript stream, and SHALL NOT jump, republish, or otherwise mutate the OUTPUT transcript's scroll position — the viewer's submitted words appear in the INPUT history only, never doubled into the agent-authored transcript.

The viewer turn SHALL be authored by the runtime at submit time and SHALL be distinguishable from agent-authored transcript units by a dedicated viewer turn kind. The output-publication contract addresses the OUTPUT transcript only: an adapter MUST NOT author, publish, or otherwise forge a viewer turn through the output-publication contract, an adapter has no path to write into the INPUT history at all, and a publish that attempts to use the viewer turn kind SHALL be rejected. The echoed viewer turn SHALL carry the submission's content classification and SHALL obey the same redaction, safe-mode, freeze, and Bounded Transcript Viewport rules as agent-authored transcript content — it is not automatically safe because it is the viewer's own text. The viewer echo is local-first presentation, not a new attention event: it SHALL NOT increment the portal's unread-output count and SHALL NOT escalate interruption class, because the viewer has by definition already seen their own message, consistent with the Ambient Portal Attention Defaults requirement. The echo is a presentation of an already-accepted submission and SHALL NOT alter the existing submission contract: the submitted text SHALL still be delivered transactionally to the adapter's semantic input mechanism per the Cooperative Projection Input Mapping requirement, and a submission that is rejected SHALL NOT be echoed. Visual differentiation of viewer versus agent turns (the two-region layout, alignment, role accent, attribution affordance, and divider treatment) is governed by the portal's component-profile and design tokens and is not mandated at the pixel level by this requirement; the requirement establishes that viewer turns are first-class, kind-distinct units of the INPUT history, held separately from the agent-authored OUTPUT transcript.

Source: RFC 0013 §3.3 and §4.3, `about/heart-and-soul/vision.md` ("a persistent on-screen portal where a person can converse"), `about/heart-and-soul/presence.md` (Interaction — local-first), owner live round-6 decision (2026-07-04, hud-egf39 / PR #1038: "route viewer submissions to INPUT-pane history, not OUTPUT transcript"), `crates/tze_hud_projection/src/contract.rs` (OutputKind::Viewer), `crates/tze_hud_projection/src/authority.rs` (append_viewer_echo on submit_portal_input), `crates/tze_hud_runtime/src/windowed/portal.rs` (append_raw_tile_viewer_echo → viewer_echo_queue → compositor viewer-echo stack, #1020/hud-hsc1t), `crates/tze_hud_runtime/src/portal_projection_driver.rs` (parse_output_kind rejects adapter-supplied viewer), exemplar `text_stream_portal_exemplar.py` (append_input_history records into input_history, never body_full)
Scope: v1-mandatory

#### Scenario: accepted reply appears in the input history

- **WHEN** a viewer submits a reply that the portal accepts
- **THEN** the submitted text SHALL appear in the portal's INPUT history as a viewer-authored, kind-distinct turn beneath the composer, stacked with token-styled turn dividers
- **AND** the submission SHALL still be delivered transactionally to the adapter's semantic input mechanism per the existing Cooperative Projection Input Mapping requirement

#### Scenario: viewer submission never enters the output transcript

- **WHEN** a viewer reply is echoed into the INPUT history
- **THEN** the submitted text SHALL NOT be appended to the OUTPUT/agent transcript stream
- **AND** the OUTPUT transcript's scroll position SHALL NOT jump or republish as a side effect of the submission
- **AND** the viewer's words SHALL appear once, in the INPUT history only, never doubled into the agent-authored transcript

#### Scenario: viewer echo does not count as unread or escalate attention

- **WHEN** a viewer reply is echoed into the INPUT history
- **THEN** the portal's unread-output count SHALL NOT increase
- **AND** the echo SHALL NOT raise the portal's interruption class beyond the ambient default

#### Scenario: adapter cannot forge a viewer turn

- **WHEN** an adapter publishes transcript output using the viewer turn kind
- **THEN** the runtime SHALL reject the publish
- **AND** the adapter SHALL have no path to write into the INPUT history; only the runtime's submit path SHALL author viewer turns

#### Scenario: viewer echo redacts like transcript content

- **WHEN** the current viewer's policy redacts the portal's transcript
- **THEN** the echoed viewer turn in the INPUT history SHALL be redacted under the same policy as agent-authored content
- **AND** it SHALL NOT bypass viewer-class filtering because it is the viewer's own submitted text

#### Scenario: rejected submission is not echoed

- **WHEN** a viewer submission is rejected (for example because the HUD is unavailable or the input queue is full)
- **THEN** no viewer turn SHALL be appended to the INPUT history
- **AND** the existing rejection feedback SHALL convey why the submission did not land
