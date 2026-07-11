# text-stream-portals Delta: conversational turn model + per-turn role attribution

## ADDED Requirements

### Requirement: Conversational Turn Model and Per-Turn Role Attribution

The OUTPUT transcript of a text stream portal SHALL be presented as a sequence
of discrete conversational turns — one per retained `TranscriptUnit` — rather
than as an opaque single blob of text. Adjacent turns SHALL be visually
separated by a token-styled turn divider, and each turn SHALL carry token-styled
role attribution derived from its runtime-assigned output kind, so a viewer can
distinguish the assistant's own conversational turns from tool, status, error,
or other agent-side output within the same OUTPUT transcript.

Attribution SHALL resolve entirely from design tokens through the portal's
component-profile path and MUST NOT use any hardcoded compositor color: the
assistant's own turns present in the base transcript text color, and non-
assistant agent-side turns (tool, status, error, other) present in a distinct
attribution token color. Attribution is derived solely from the runtime-assigned
`output_kind` of each unit and MUST NOT be forgeable by adapter-supplied content;
it is presentation of metadata the runtime already retains and MUST NOT require
adapter cooperation. This requirement governs the agent-authored OUTPUT
transcript only and does not alter the two-history INPUT/OUTPUT split of the
Viewer Reply Echo requirement — viewer turns remain first-class, kind-distinct
units of the separately-bounded INPUT history and are not attributed by this
requirement.

Per-turn attribution is ambient, subordinate presentation consistent with the
Ambient Portal Attention Defaults requirement: it MUST NOT escalate the portal's
interruption class and MUST NOT count as unread. Attribution is content-adjacent
and SHALL obey the same redaction, safe-mode, freeze, and Bounded Transcript
Viewport rules as the transcript text it decorates — a restricted viewer whose
`visible_transcript` is emptied upstream sees no turns and therefore no
attribution, so attribution discloses nothing a redacted viewer could not
already see.

Turn presentation MUST preserve the existing coalescing invariant: per-render
turn presentation MUST NOT introduce a per-node `AddNode` fan-out that would
reclassify the hot transcript-republish batch as Transactional and defeat
StateStream latest-wins coalescing (Coherent Transcript Coalescing requirement).
Until a compositor vertical-flow layout capability exists to position per-turn
scene nodes, the turn model SHALL be carried within the single coalescible
transcript node (turn dividers in the transcript markdown, attribution as
token-resolved color-run spans over each attributed turn's text). Materializing
each turn as its own scene node via inline `NodeProto.children` is scene-mutation
schema work permitted only once portal promotion is gated (Promotion Scope
Boundary requirement) and additionally gated on that vertical-flow layout
capability; a future structural node split SHALL satisfy this same contract
without re-opening the attribution decision.

Source: RFC 0013 §4.3, hud-0yrix audit (`vd-no-conversational-turn-model` /
`chat-no-turn-structure-attribution`), §Viewer Reply Echo, §Ambient Portal
Attention Defaults, §Portal Component Profile Styling, §Promotion Scope Boundary,
turn separators from hud-nx7yq.4 (`crates/tze_hud_projection/src/resident_grpc.rs`
`visible_transcript_markdown_with`), `crates/tze_hud_projection/src/contract.rs`
(`OutputKind`), PR #1149 / hud-ga4md (`NodeProto.children` materialization)
Scope: v1

#### Scenario: adjacent turns are separated and role-attributed

- **WHEN** an expanded portal renders a retained OUTPUT transcript window with more than one turn
- **THEN** adjacent turns SHALL be separated by a token-styled turn divider
- **AND** each non-assistant agent-side turn (tool, status, error, other) SHALL be presented in the distinct attribution token color rather than the base assistant transcript color
- **AND** assistant-authored turns SHALL present in the base transcript text color

#### Scenario: attribution derives from runtime output kind, not adapter content

- **WHEN** the portal attributes a turn's role
- **THEN** the role SHALL derive from that unit's runtime-assigned `output_kind`
- **AND** adapter-supplied transcript text SHALL NOT be able to change or forge the attributed role

#### Scenario: attribution resolves from tokens, not hardcoded color

- **WHEN** the portal presents per-turn role attribution
- **THEN** every attribution color SHALL resolve from the active design tokens through the component-profile path
- **AND** no attribution color SHALL come from a hardcoded compositor value

#### Scenario: attribution stays ambient and redaction-safe

- **WHEN** per-turn attribution is present in the transcript
- **THEN** it SHALL NOT raise the portal's interruption class and SHALL NOT count as unread
- **AND** when the current viewer's policy redacts the portal, the emptied transcript SHALL present no turns and therefore no attribution

#### Scenario: turn attribution does not break coalescing

- **WHEN** an expanded portal republishes its transcript on each append under state-stream backpressure
- **THEN** the turn model SHALL be carried on the single coalescible transcript node without a per-turn `AddNode` fan-out
- **AND** the republish batch SHALL remain a coalescible StateStream update rather than being reclassified Transactional
