# text-stream-portals Delta: vertical-flow transcript layout

## ADDED Requirements

### Requirement: Vertical-Flow Transcript Layout

The runtime SHALL be able to lay out a stacked sequence of transcript child nodes
by VERTICAL FLOW: measuring each child's wrapped rendered height at its own width
and positioning each subsequent child directly below the previous one plus an
inter-child gap, so a multi-node transcript body (one node per conversational
turn) can be materialized without the publisher measuring wrapped text. The
measured child height SHALL derive from the same wrapped-line shaping the render
path uses, so a child's measured row height equals its painted row height. For a
markdown transcript node this specifically means the SAME markdown parse and
per-span shaping the render path applies to a `TextMarkdownNode` — including
markdown syntax stripped before wrapping and any heading font-size-scale spans —
not a generic plain-text measurement of the raw source (hud-3xdlf). A transcript
node carrying at least one pixel-bearing color run (e.g. hud-26869 per-turn role
attribution) is a SEPARATE render path — the render path SKIPS markdown stripping
for such a node so the color run's pinned byte offsets stay valid — and
measurement SHALL mirror that fork: RAW (un-stripped) content, the node's own
font family, and the default line-height multiplier (that render path has no
access to a resolved token set either, by the same offset-pinning constraint),
not the parsed/token-resolved measurement used for the common case (hud-ysyis).

Vertical-flow layout resolution SHALL run in the runtime/compositor and MUST NOT
require the owning model or adapter to compute child positions — consistent with
the doctrine that the model drives the scene while the runtime composits
(LLM-out-of-frame-loop). The inter-child gap SHALL be sourced from a design token
via the portal's component profile (`portal.spacing.section_gap_px`) and MUST NOT
be a hardcoded compositor value.

Vertical-flow layout is additive and default-off: a node whose children are not
marked for flow SHALL continue to be positioned by each child's explicit bounds
exactly as before, so the single-node raw-tile transcript path is byte-identical
and existing publishers are unaffected.

Materializing the OUTPUT transcript as one scene node per turn on top of this
layout is promotion-era work and remains gated on the Phase-1 Promotion Evidence
Gate: until that gate passes, the raw-tile pilot's single-node transcript stays
authoritative and this capability is exercised as the runtime resolution engine
that a future per-turn split will consume. The split, when it lands, SHALL remain
within the Promotion Scope Boundary (existing node types, no new transport, no
scene-graph transcript history — only the bounded visible window is materialized).

Source: hud-txkbh, hud-26869 (deferred structural half of the turn model),
hud-3xdlf (markdown-aware measurement), hud-ysyis (raw-content/color-run
measurement mode + real-token threading), PR #1149 / hud-ga4md
(`NodeProto.children` materialization), §Phase-0 Raw-Tile Pilot, §Phase-1
Promotion Evidence Gate, §Promotion Scope Boundary, §Portal Component Profile
Styling, `crates/tze_hud_compositor/src/vertical_flow.rs`,
`crates/tze_hud_compositor/src/text.rs` (`composer_wrap_line_widths` for
plain-text children, `measure_markdown_content_height` for markdown children
without pixel-bearing color runs, `measure_raw_content_height` for markdown
children WITH them), `crates/tze_hud_config/src/portal_tokens.rs`
(`section_gap_px`)
Scope: v1

#### Scenario: flowed children stack without overlap

- **WHEN** the runtime resolves a vertical-flow layout for an ordered set of transcript child nodes
- **THEN** each child SHALL be positioned directly below the previous child plus the inter-child gap
- **AND** no child's resolved vertical band SHALL overlap another's

#### Scenario: child height derives from render-path shaping

- **WHEN** the runtime measures a flowed child's height for layout
- **THEN** the height SHALL derive from the same wrapped-line shaping the render path uses for that content and width
- **AND** a child that wraps to more visual lines SHALL resolve to a taller row than the same content on a wider child

#### Scenario: attributed transcript nodes measure on the same raw basis they paint on

- **WHEN** the runtime measures a flowed child whose `TextMarkdownNode` carries at least one pixel-bearing color run
- **THEN** the height SHALL derive from the RAW (un-stripped) content, the node's own font family, and the default line-height multiplier — the same basis that node's render path paints on — and NOT from a markdown-parsed/token-resolved measurement
- **AND** for a child without any pixel-bearing color run, the height SHALL derive from the markdown-parsed, token-resolved measurement instead

#### Scenario: layout runs in the runtime, not the publisher

- **WHEN** a publisher emits a stacked multi-node transcript body without child vertical positions
- **THEN** the runtime SHALL resolve each child's vertical position by measuring and stacking
- **AND** the owning model or adapter SHALL NOT be required to compute child positions

#### Scenario: inter-child gap is token-sourced, not hardcoded

- **WHEN** the runtime stacks flowed children
- **THEN** the inter-child gap SHALL resolve from the active design tokens via the portal's component profile
- **AND** no gap value SHALL come from a hardcoded compositor constant

#### Scenario: default-off preserves single-node behavior

- **WHEN** a node's children are not marked for vertical flow
- **THEN** each child SHALL be positioned by its own explicit bounds exactly as before this change
- **AND** the raw-tile single-node transcript rendering SHALL be byte-identical

#### Scenario: per-turn node materialization stays gated

- **WHEN** promotion-era work proposes materializing the OUTPUT transcript as one scene node per turn driven by this layout
- **THEN** that split SHALL remain gated on the Phase-1 Promotion Evidence Gate
- **AND** until the gate passes, the raw-tile pilot's single-node transcript SHALL stay authoritative
