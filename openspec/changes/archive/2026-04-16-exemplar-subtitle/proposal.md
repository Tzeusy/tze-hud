## Why

The subtitle zone is the most visible and most frequently used publishing surface in tze_hud. Every agent that speaks to the user — transcription, narration, status updates — publishes through `subtitle`. Yet today there is no reference implementation that demonstrates the full subtitle rendering pipeline end-to-end: design-token-driven white-on-black-outline text, semi-transparent backdrop, streaming word-by-word reveal, fade transitions, rapid replacement without flicker, and auto-clear after TTL. Without a concrete exemplar, the component-shape-language spec remains abstract — implementers have no canonical target to build toward and testers have no fixture to validate against.

A polished subtitle exemplar serves three purposes: (1) it is the visual proof that the design token system, RenderingPolicy extensions, and zone rendering pipeline work together correctly, (2) it defines the exact MCP publish sequences that exercise every subtitle behavior (single line, multi-line overflow, rapid replacement, streaming with breakpoints, TTL expiry), and (3) it provides ready-made user-test fixtures that plug into the existing cross-machine validation workflow (`/user-test` skill).

## What Changes

- Define a **subtitle zone exemplar specification** covering the complete visual contract: token-resolved RenderingPolicy fields, DualLayer readability enforcement, 8-direction outline rendering, backdrop opacity, fade transitions, and word-wrap/overflow behavior.
- Define **behavioral test scenarios** for all subtitle zone interactions: stream-text publish, rapid replacement, TTL auto-clear, multi-line overflow with ellipsis, streaming word-by-word reveal with breakpoints.
- Define **MCP test fixtures** — concrete `publish_to_zone` call sequences in JSON format that exercise each scenario, compatible with the existing `publish_zone_batch.py` script and `/user-test` skill.
- Define a **user-test scenario** where an agent publishes a subtitle sequence via MCP and each message renders with correct token-driven styling.

## Capabilities

### New Capabilities
- `exemplar-subtitle`: Production-quality subtitle zone exemplar defining the visual contract, behavioral scenarios, MCP test fixtures, and user-test integration for the subtitle zone — the flagship zone exemplar.

### Modified Capabilities

(none — this exemplar defines test/validation artifacts for existing capabilities, it does not change spec-level requirements)

## Impact

- **Test fixtures**: New JSON test message files for subtitle-specific scenarios in `.claude/skills/user-test/scripts/`.
- **User-test workflow**: Subtitle-specific test scenario added to the existing cross-machine validation skill.
- **Implementer guidance**: Concrete rendering targets and acceptance criteria for compositor subtitle rendering (outline, backdrop, transitions, streaming).
- **No code changes**: This exemplar defines the target — implementation is driven by the component-shape-language tasks (RenderingPolicy extensions, token system, compositor refactoring).
