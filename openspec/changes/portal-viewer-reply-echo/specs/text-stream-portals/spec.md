## ADDED Requirements

### Requirement: Viewer Reply Echo

When a viewer submits a reply through a text stream portal composer and the submission is accepted, the runtime SHALL echo the submitted text into the portal's retained transcript as a viewer-authored turn, so the two-way conversation is visible on the surface rather than the viewer's own words disappearing into the adapter inbox. The viewer turn SHALL be authored by the runtime at submit time and SHALL be distinguishable from agent-authored transcript units by a dedicated viewer turn kind; an adapter MUST NOT author, publish, or otherwise forge a viewer turn through the output-publication contract, and a publish that attempts to use the viewer turn kind SHALL be rejected. The echoed viewer turn SHALL carry the submission's content classification and SHALL obey the same redaction, safe-mode, freeze, and Bounded Transcript Viewport rules as agent-authored transcript content — it is not automatically safe because it is the viewer's own text. The viewer echo is local-first presentation, not a new attention event: it SHALL NOT increment the portal's unread-output count and SHALL NOT escalate interruption class, because the viewer has by definition already seen their own message, consistent with the Ambient Portal Attention Defaults requirement. The echo is a presentation of an already-accepted submission and SHALL NOT alter the existing submission contract: the submitted text SHALL still be delivered transactionally to the adapter's semantic input mechanism per the Cooperative Projection Input Mapping requirement, and a submission that is rejected SHALL NOT be echoed. Visual differentiation of viewer versus agent turns (alignment, role accent, attribution affordance) is governed by the portal's component-profile and design tokens and is not mandated at the pixel level by this requirement; the requirement establishes that viewer turns are first-class, kind-distinct units within the bounded retained transcript window.

Source: RFC 0013 §3.3 and §4.3, `about/heart-and-soul/vision.md` ("a persistent on-screen portal where a person can converse"), `about/heart-and-soul/presence.md` (Interaction — local-first), `crates/tze_hud_projection/src/contract.rs` (OutputKind::Viewer), `crates/tze_hud_projection/src/authority.rs` (append_viewer_echo on submit_portal_input), `crates/tze_hud_runtime/src/portal_projection_driver.rs` (parse_output_kind rejects adapter-supplied viewer)
Scope: v1-mandatory

#### Scenario: accepted reply appears as a viewer turn

- **WHEN** a viewer submits a reply that the portal accepts
- **THEN** the submitted text SHALL appear in the retained visible transcript as a viewer-authored, kind-distinct turn
- **AND** the submission SHALL still be delivered transactionally to the adapter's semantic input mechanism per the existing Cooperative Projection Input Mapping requirement

#### Scenario: viewer echo does not count as unread or escalate attention

- **WHEN** a viewer reply is echoed into the transcript
- **THEN** the portal's unread-output count SHALL NOT increase
- **AND** the echo SHALL NOT raise the portal's interruption class beyond the ambient default

#### Scenario: adapter cannot forge a viewer turn

- **WHEN** an adapter publishes transcript output using the viewer turn kind
- **THEN** the runtime SHALL reject the publish
- **AND** only the runtime's submit path SHALL author viewer turns

#### Scenario: viewer echo redacts like transcript content

- **WHEN** the current viewer's policy redacts the portal's transcript
- **THEN** the echoed viewer turn SHALL be redacted under the same policy as agent-authored content
- **AND** it SHALL NOT bypass viewer-class filtering because it is the viewer's own submitted text

#### Scenario: rejected submission is not echoed

- **WHEN** a viewer submission is rejected (for example because the HUD is unavailable or the input queue is full)
- **THEN** no viewer turn SHALL be appended to the transcript
- **AND** the existing rejection feedback SHALL convey why the submission did not land
