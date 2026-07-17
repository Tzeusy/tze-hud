# Phase-1 Promotion Evidence Session — 2026-07-17

Bead: **hud-clu38** · Gate: `openspec/specs/text-stream-portals/spec.md` §Phase-1
Promotion Evidence Gate · Owner promotion approval granted 2026-07-17 (recorded on
hud-uym23). Target: tzehouse reference Windows host (engineering-bar §2 reference
identity). Collector: Linux rig via user-test skill.

Passing this gate unblocks: hud-uym23 (per-turn transcript node split) and the seven
promotion-scoped requirements (per-turn delivery acks, unread divider + count,
jump-to-latest, ambient timestamps, activity cue, first-run treatment, connecting-state
distinction).

## Track A — Automated (agent-run, this session)

1. **Build + deploy**: `scripts/build_windows_app.sh --profile release` from main,
   deploy via `.claude/skills/user-test/subskills/portal-hud-deploy/scripts/deploy_portal_hud.sh --local-exe <exe>`
   (TzeHudOverlay scheduled task, exe-direct, overlay mode). Record commit, sha256, ports.
2. **Exemplar adapter family — six gate axes** via
   `text_stream_portal_exemplar.py --phases markdown,overflow,composer-edit,cadence,profile-swap,window-mgmt`
   → structured evidence artifact (reference-hardware tag; cadence axis carries
   publish→present vs transport-RTT overhead per spec).
3. **Cooperative projection adapter family + agent-ergonomics demo**: this LLM session
   attaches via the vendored hud-projection skill surface only
   (`portal_client.py`: attach → publish stream → publish_status → long-poll
   get_pending_input → acknowledge → detach), zero scene-graph mutations in LLM
   context. Ceremony recorded: operation count + any glue needed outside the skill.
4. **Raw-tile complexity observations**: tile counts / mutation batch shapes from the
   exemplar transcript (recurring-complexity evidence).
5. **Governance confirmation**: redaction/safe-mode/freeze covered by the
   `text_stream_portal_governance.rs` integration suite on this commit (CI); live axis
   here = **orphan path**: kill the exemplar client mid-lease, observe stale/degraded
   treatment and lease-grace removal, then confirm a fresh attach starts a fresh portal.
6. **Screenshots** per axis via the TzeHudCapture scheduled task.
   ⚠ Capture minimizes all non-tze_hud windows — coordinate with the operator before
   each shot. Screendump→physical mapping per multimon notes (primary = dump rect
   (0,1440)-(2560,2880), phys = dump×1.5, y−2160).

## Track B — Human keyboard round (operator at tzehouse)

Synthetic key injection cannot produce Unicode keystrokes/submits on this host; these
need real keys. One sitting clears the gate's live composer-editing evidence AND the
standing live-verify beads:

| # | Do | Expect | Bead |
|---|---|---|---|
| 1 | Click the portal composer; type a long paragraph past the pane width | draft soft-wraps into multiple rows; caret tracks | gate: composer editing |
| 2 | Ctrl+Enter twice mid-draft, keep typing, then Enter to submit | newlines render in draft; submit clears composer; entry appears in INPUT history | gate + hud-pncm3 |
| 3 | Inspect the submitted entry in INPUT history | long entry WRAPS to pane width; embedded newlines render as line breaks | hud-pncm3 |
| 4 | Submit ~10 more entries until history overflows | newest entry stays visible (tail-anchored) | hud-jezmt context |
| 5 | Wheel-scroll up over the INPUT pane, then type | history scrolls back; typing jumps back to caret | hud-acfvp |
| 6 | Press Tab repeatedly (6+), watch focus ring; type at each stop | visible ring at every stop; printable key refocuses composer + inserts; Esc returns to composer; typing never goes dead | hud-2v8br |
| 7 | Ctrl+= / Ctrl+- several times; drag header to move | portal grows/shrinks anchored top-left, text re-wraps, background stays opaque | hud-sp8l7, hud-w41ef live-confirm |
| 8 | Hover edges/corners + header of the FOCUSED portal | resize/move cursor shapes appear over affordance bands | hud-gmwuf |
| 9 | Watch a streaming reply + scroll during it | easing/scroll feel: note anything sluggish/jittery (motion cadence tuning) | hud-pdl1d |
| 10 | Composer polish sweep: Home/End/arrow-up-down across wrapped rows, select+paste | caret/selection behave; paste lands at caret | hud-vvdvy |
| 11 | Reply to the attached agent-ergonomics session when it asks | delivery cue: → sending then ✓✓ delivered on your echo | gate: cooperative adapter input leg |

Per-turn attribution live check (hud-2u5j7): during Track A step 3 the agent publishes
tool/sub-agent attributed turns — operator confirms color distinction in OUTPUT pane.

## Cleanup (mandatory before wind-down)

- Clear any ambient/zone test publishes (no green screens left behind).
- Detach the cooperative projection session; dismiss exemplar tiles (release lease).
- Truncate/rotate hud-diag.log if ballooned; restore operator windows after captures.

## Artifact index (filled as produced)

- `exemplar-evidence.json` — six-axis exemplar artifact (reference tag)
- `agent-ergonomics-ceremony.md` — op-by-op ceremony log, cooperative adapter
- `orphan-path.md` — governance live axis notes
- `shots/` — per-axis screenshots
- `human-round.md` — Track B operator results (fill per table row)
