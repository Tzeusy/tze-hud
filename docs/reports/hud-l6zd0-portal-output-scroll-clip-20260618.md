# Portal Output Scroll Clip Review

Bead: `hud-l6zd0`
PR: `#914`
Date: 2026-06-18

## Symptom

Live Windows observation reported flickering phantom black rectangles above and
below the text-stream portal OUTPUT pane while the transcript tile was scrolled.
The expected behavior is that root fills, scrolled child fills, hit-region
tints, text/image backgrounds, and retained rounded fills remain clipped to the
OUTPUT tile viewport at every scroll offset.

## Fix Shape

The compositor now clips tile-local flat geometry before emitting draw vertices
for solid-color fills, text backgrounds/code panels, hit-region feedback tints,
static-image placeholders, and textured image draws. Textured-image clipping
also adjusts UV bounds when only part of the image remains inside the tile.

Retained rounded solid-color commands preserve the original rounded rectangle
shape used by the SDF and carry a separate clipped raster quad for the tile
viewport. This prevents overdraw outside the output viewport without introducing
new rounded corners at the clipped edge.

## Verification

- `cargo fmt -p tze_hud_compositor --check` — pass.
- `cargo check -p tze_hud_compositor --features headless` — pass.
- `cargo clippy -p tze_hud_compositor --all-targets --features headless -- -D warnings` — pass.
- `HEADLESS_FORCE_SOFTWARE=1 cargo test -p tze_hud_compositor --features headless scrolled_ -- --nocapture` — pass. This ran the portal-like headless readback at `scroll_y = 0, 48, 96` and the rounded-SDF shape/clip regression.
- `HEADLESS_FORCE_SOFTWARE=1 cargo test -p integration --test text_stream_portal_surface -- --nocapture` — pass. This preserves functional portal scrolling and append-while-scrolled behavior.

## Live Evidence Status

A reviewer-worker live scroll attempt reached the resident gRPC endpoint at
`windows-host.example:50051`, but authentication failed with
`AUTH_FAILED` because the local `TZE_HUD_PSK` no longer matched the scheduled
task's current PSK. No post-fix operator-visible scroll sign-off was collected
in this review pass.

If the original manual `/user-test` criterion remains required for closure,
recover the current `TzeHudOverlay` PSK from the Windows scheduled task using
the private host mapping, then rerun:

```bash
python3 .claude/skills/user-test/scripts/text_stream_portal_exemplar.py \
  --target <windows-host>:50051 \
  --psk-env TZE_HUD_PSK \
  --agent-id agent-alpha \
  --doc docs/reports/exemplar-manual-review-checklist.md \
  --phases scroll \
  --max-lines 80 \
  --transcript-out test_results/text-stream-portal-scroll-postfix.json
```
