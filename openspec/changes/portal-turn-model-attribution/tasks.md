# Tasks: portal-turn-model-attribution

## 1. Spec delta (this change's deliverable)

- [x] 1.1 Author delta: Conversational Turn Model and Per-Turn Role Attribution
- [x] 1.2 `openspec validate portal-turn-model-attribution --strict` passes
- [ ] 1.3 Commit + push (bead hud-26869 branch → PR)

## 2. Implementation (bead hud-26869)

- [x] 2.1 Add `transcript_system_color` attribution token: `PortalPartTokens`
      field + `PORTAL_TOKEN_TRANSCRIPT_SYSTEM_COLOR` key + default +
      `resolve_portal_tokens` resolution (`crates/tze_hud_config/src/portal_tokens.rs`).
- [x] 2.2 Plumb the token into `PortalVisualTokens` +
      `portal_visual_tokens_from_part_tokens` mapping
      (`crates/tze_hud_projection/src/resident_grpc.rs`).
- [x] 2.3 Segment the OUTPUT transcript body into per-turn byte spans keyed on
      `OutputKind` at assembly time; emit token-resolved `TextColorRunProto`
      attribution spans for non-assistant turns in `portal_node`. Keep the
      single node (`children: vec![]`); no `AddNode` fan-out.
- [x] 2.4 Unit tests: segmentation offsets correct against emitted content;
      attribution runs cover only non-assistant turns; absent for all-assistant
      and empty/collapsed/redacted; runs carry the token (no literal color);
      still a single node.

## 3. Closeout

- [ ] 3.1 File follow-up: compositor vertical-flow layout capability +
      true per-turn scene-node transcript (blocked-on-layout), so a future
      promotion can satisfy this contract structurally.
- [ ] 3.2 Sync + archive per hud-hpuzp convention once implementation merges.
- [ ] 3.3 Live re-verify on reference Windows overlay (attribution colors read
      distinctly; separators intact) — hardware-gated.
