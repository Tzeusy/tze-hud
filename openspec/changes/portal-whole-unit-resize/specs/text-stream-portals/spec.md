## MODIFIED Requirements

### Requirement: Portal Window Management

Expanded portal surfaces SHALL support viewer-driven move and resize with local-first feedback. Move continues through the existing header drag affordance. Move and resize SHALL operate on the portal as a single coherent unit: when a portal is composed of multiple constituent surfaces (e.g. frame, transcript pane, composer/input pane, drag affordances), all constituent surfaces SHALL move and scale together preserving their relative layout, and focusing a constituent surface (such as the composer/input pane) SHALL move or resize the WHOLE portal — never an individual constituent surface in isolation. Resize SHALL be available through both (a) pointer-driven resize affordances on the portal frame (corner or edge capture regions in the content layer) and (b) focus-scoped keyboard shortcuts: while the portal surface holds keyboard focus, Ctrl+`+` (and its unshifted form Ctrl+`=`) SHALL grow and Ctrl+`-` SHALL shrink the portal by a token-defined step. Shortcuts MUST be focus-scoped: a portal that does not hold focus MUST NOT consume them, chrome- and shell-reserved shortcuts take precedence, and safe-mode input capture overrides them entirely. Resize-shortcut handling SHALL be robust to release-only key delivery: when the host input source delivers the resize chord as a key release with no preceding matching resize key press — as Windows `SendInput` does for a held-modifier `=`/`-` chord — the runtime SHALL apply the resize step on the key release as a fallback, and SHALL deduplicate consumed press/release pairs so a normal physical key-down/key-up cycle resizes exactly once. Geometry feedback during a move or resize gesture SHALL render locally within the input-to-local-ack budget (`about/craft-and-care/engineering-bar.md` §2); the owning adapter SHALL observe geometry changes only as coalescible state-stream snapshots and MUST NOT veto or reposition the surface mid-gesture. Resize SHALL clamp to token-defined minimum legible bounds and to maxima within the portal's lease bounds and scene budgets. At every intermediate and final geometry, pane layout SHALL re-resolve under the Transcript Overflow and Ellipsis Contract: no partially clipped glyphs. When transcript or composer content overflows its pane, the surface SHALL show a token-styled scroll-position indicator; the indicator conveys geometry only and SHALL remain present under redaction without revealing content.

Source: RFC 0013 §4.1 and §4.2, RFC 0004 (input model, focus, key press/release semantics), `about/craft-and-care/engineering-bar.md` §2, CLAUDE.md core rules "local feedback first" and "screen is sovereign"

#### Scenario: pointer resize is local-first

- **WHEN** the viewer drags a portal resize affordance
- **THEN** the portal's geometry SHALL update locally within the input-to-local-ack budget for the duration of the gesture
- **AND** the owning adapter SHALL observe the geometry change only through coalescible state-stream snapshots after the fact

#### Scenario: resize affects the whole portal, not a focused sub-surface

- **WHEN** the viewer focuses a constituent surface of a multi-surface portal (e.g. the composer/input pane) and applies a resize shortcut or pointer resize affordance
- **THEN** the entire portal SHALL grow or shrink as a unit, with all constituent surfaces scaling together and preserving their relative layout
- **AND** no individual constituent surface SHALL resize or reposition independently of the portal

#### Scenario: focused portal responds to resize shortcuts

- **WHEN** a portal surface holds keyboard focus and the viewer presses Ctrl+`+` or Ctrl+`-`
- **THEN** the portal SHALL grow or shrink by the token-defined step with local-first feedback
- **AND** pane layout SHALL re-resolve without partially clipped glyphs at the new geometry

#### Scenario: unfocused portal ignores resize shortcuts

- **WHEN** the viewer presses Ctrl+`+` or Ctrl+`-` while no portal surface holds keyboard focus
- **THEN** no portal SHALL change size
- **AND** the key events SHALL remain available to chrome and other focus targets per the existing input-routing contract

#### Scenario: resize is robust to release-only key streams

- **WHEN** a focused portal receives a resize chord (Ctrl+`=`/`+` or Ctrl+`-`) as a key release with no preceding matching resize key press, as occurs with Windows `SendInput` for a held-modifier chord
- **THEN** the runtime SHALL apply the resize step on the key release as a fallback so the focused portal still grows or shrinks
- **AND** a normal physical key-down/key-up cycle SHALL resize exactly once, because consumed press/release pairs are deduplicated rather than double-applied

#### Scenario: resize clamps to bounds

- **WHEN** repeated shrink or grow operations are applied past the configured limits
- **THEN** the portal SHALL clamp at the token-defined minimum legible bounds and at the maxima permitted by its lease bounds and scene budgets
- **AND** no intermediate geometry SHALL render partially clipped glyphs

#### Scenario: adapter cannot override an active gesture

- **WHEN** the owning adapter publishes portal content or geometry while a viewer move or resize gesture is in progress
- **THEN** the viewer's gesture SHALL remain authoritative for surface geometry until the gesture ends
- **AND** the adapter's content updates SHALL apply within the gesture-defined geometry

#### Scenario: scroll-position indicator is geometry-only under redaction

- **WHEN** transcript content overflows its pane for a viewer whose policy redacts the portal's content
- **THEN** the token-styled scroll-position indicator SHALL remain present and reflect scroll position
- **AND** the indicator SHALL NOT convey transcript content beyond geometry

## ADDED Requirements

### Requirement: Portal Resize Text Scaling

Whole-portal resize SHALL scale the portal's text with the portal: on a group grow or shrink, per-node font sizes SHALL scale by the portal's scale ratio, clamped to token-defined minimum and maximum legible sizes, with pane layout re-resolving (wrap, overflow, ellipsis) at the scaled font within the new geometry. Font scaling is viewer-local presentation: it MUST NOT require adapter cooperation and MUST NOT alter the adapter-published content or its logical structure. When the horizontal and vertical scale ratios differ, the text scale SHALL derive from a single deterministic ratio (the width ratio) so glyphs never distort anisotropically.

Source: owner live direction 2026-07-03 ("I kinda wish text shrunk and grew with it", hud-ovjxu), supersedes the constant-font reading of the reflow clause above
Scope: v1

#### Scenario: growing the portal grows the text

- **WHEN** the viewer grows a portal via the resize affordances or hotkeys
- **THEN** transcript and composer text SHALL render at a proportionally larger font, clamped to the token-defined maximum
- **AND** the layout SHALL re-resolve at the new font and geometry with no partially clipped glyphs

#### Scenario: shrinking clamps at legible minimum

- **WHEN** the viewer shrinks a portal such that proportional scaling would drop below the token-defined minimum legible size
- **THEN** the font SHALL clamp at the minimum and further shrink SHALL reduce only the visible content window
