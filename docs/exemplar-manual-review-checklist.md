# Exemplar Manual Review Checklist

Manual UX/design review of all tze_hud exemplars.
Reviewed: 2026-04-06

## Exemplars

| # | Exemplar | Status | Notes |
|---|----------|--------|-------|
| 1 | [Subtitle](#1-subtitle) | **done** | Removed backdrop, fixed centering, fixed streaming reveal speed, added fade transitions |
| 2 | [Alert Banner](#2-alert-banner) | **done** | Fixed missing text, added 1px outline, taller banners |
| 3 | [Notification](#3-notification) | **deferred** | Basic rendering works; needs modern redesign (rounded corners, icons, dismiss, action buttons) |
| 4 | [Status Bar](#4-status-bar) | **deferred** | Repositioned to right edge, compact value-only format. Needs SVG widget redesign (hud-x2v1) |
| 5 | [Ambient Background](#5-ambient-background) | **deferred** | Fixed exemplar alpha 1.0→0.15; StaticImage rendering blocked on hud-4sk5 |
| 6 | [Dashboard Tile](#6-dashboard-tile) | pending | gRPC test client built; ready for visual review |
| 7 | [Presence Card](#7-presence-card) | pending | |
| 8 | [Gauge Widget](#8-gauge-widget) | pending | |
| 9 | [Progress Bar](#9-progress-bar) | pending | |
| 10 | [Status Indicator](#10-status-indicator) | pending | |

---

## 1. Subtitle

**Type:** Zone | **Contention:** latest-wins | **Position:** bottom-center

**Spec:** `openspec/changes/exemplar-subtitle/`
**Profile:** `profiles/exemplar-subtitle/`

### Design Review

- [ ] Text size and readability
- [ ] Backdrop opacity and color
- [ ] Word-wrap behavior at screen edge
- [ ] Streaming reveal timing feels natural
- [ ] TTL fade-out duration
- [ ] Positioning relative to other zones
- [ ] Multi-line spacing

### UX Tweaks

1. **Removed backdrop** — subtitle now uses outline-only readability (white text + black outline on transparent background). No grey rectangle.
2. **Fixed text centering** — cosmic-text `Align::Center` was stored but never applied to glyphon buffers. Now wired through `buf.lines.set_align()`.
3. **Fixed streaming reveal speed** — `STREAM_REVEAL_FRAMES_PER_SEGMENT` increased from 1 to 10 (~167ms per word at 60fps). Previously all words revealed in <200ms total.
4. **Added fade transitions** — `transition_in_ms=200`, `transition_out_ms=150` now set in subtitle token defaults.

**Files changed:**
- `crates/tze_hud_config/src/policy_builder.rs` — removed backdrop/opacity token defaults for subtitle, added transition defaults
- `crates/tze_hud_compositor/src/text.rs` — wired `TextAlign` → `cosmic_text::Align` on buffer lines
- `crates/tze_hud_compositor/src/renderer.rs` — increased streaming reveal dwell from 1 to 10 frames

---

## 2. Alert Banner

**Type:** Zone | **Contention:** stack | **Position:** top of screen

**Spec:** `openspec/changes/exemplar-alert-banner/`
**Profile:** `profiles/exemplar-alert-banner/`

### Design Review

- [ ] Severity color contrast (info/warning/critical)
- [ ] Text legibility on colored backdrops
- [ ] Stack ordering (newest on top?)
- [ ] Auto-dismiss timing per severity
- [ ] Banner height and padding
- [ ] Animation on appear/dismiss

### UX Tweaks

1. **Fixed missing text** — `stack_slot_height` used raw `font_size_px` but cosmic-text needs `font_size * 1.4` for line_height. Text bounds were too small to fit even one line.
2. **Added 1px black text outline** — white text on amber/warning backdrop was illegible. Added `outline_color` and `outline_width=1.0` to alert-banner token defaults, wired policy outline through to notification TextItems.
3. **Taller banners** — increased `margin_vertical` to 12px and `SLOT_BASELINE_GAP` to 4px for ~20% more height.

**Files changed:**
- `crates/tze_hud_compositor/src/renderer.rs` — fixed `stack_slot_height` to use `font_size * 1.4`, wired outline through notification TextItem creation
- `crates/tze_hud_config/src/policy_builder.rs` — added outline_color, outline_width, margin_vertical, margin_horizontal defaults for alert-banner

---

## 3. Notification

**Type:** Zone | **Contention:** stack | **Position:** configurable

**Spec:** `openspec/changes/exemplar-notification/`
**Profile:** `profiles/notification-stack-exemplar/`

### Design Review

- [ ] Urgency color differentiation
- [ ] Max-depth eviction behavior
- [ ] Fade-out on TTL expiry
- [ ] Stack spacing between notifications
- [ ] Text truncation for long messages
- [ ] Multi-agent visual distinction

### UX Tweaks

Basic rendering confirmed working (colored backdrops, text, stacking, TTL expiry). However, the current design is too minimal for production use. A modern notification redesign is needed:

1. **Rounded corner rectangles** — current renderer only draws axis-aligned quads
2. **Info/urgency icons** — `icon` field exists but is stubbed (no texture pipeline in v1)
3. **Two-line layout** — title (bold) + body text (regular weight)
4. **Dismiss button (×)** — requires input routing to zones
5. **Action button** — callback semantics for notification actions

**Reference:** [Dribbble notification banner design](https://cdn.dribbble.com/userupload/14638898/file/original-3cee189d1cc549f2e3dd0471e69c8562.jpg)

**Tracked as:** bead (see below)

---

## 4. Status Bar

**Type:** Zone | **Contention:** merge-by-key | **Position:** bottom edge

**Spec:** `openspec/changes/exemplar-status-bar/`
**Profile:** `profiles/exemplar-status-bar/`

### Design Review

- [ ] Key-value pair layout and spacing
- [ ] Font choice (monospace) readability
- [ ] Bar height and padding
- [ ] Multi-agent key coexistence display
- [ ] Update coalescing visual smoothness
- [ ] Contrast against content behind bar

### UX Tweaks

_(to be filled during review)_

---

## 5. Ambient Background

**Type:** Zone | **Contention:** latest-wins | **Position:** full-screen

**Spec:** `openspec/changes/exemplar-ambient-background/`

### Design Review

- [x] Solid color rendering
- [ ] Static image scaling/positioning
- [x] Transition between backgrounds
- [x] Impact on foreground element readability
- [x] Default/fallback appearance

### UX Tweaks

1. **Fixed exemplar script alpha** — all colors changed from `a: 1.0` (fully opaque, obscures desktop) to `a: 0.15` (subtle tint). Added `DEFAULT_ALPHA` constant. Fixed log format to show actual alpha instead of hardcoded `1.0`. User confirmed 0.15 looks right for overlay mode.
2. **Increased phase 3 pause** — static image placeholder pause increased from 2s to 5s so the warm-gray is visible before phase 4 overwrites it.
3. **StaticImage renders placeholder (not acceptable for v1)** — warm-gray `(0.3, 0.3, 0.3, 1.0)` placeholder is not a real image. User requested actual GPU texture rendering. Created epic **hud-4sk5** (P1, 5 children) to implement the full wgpu texture pipeline. Spec confirms StaticImage is v1-mandatory — the placeholder was an implementation gap, not a spec gap.
4. **Instant transitions acceptable** — background swap at low alpha is not jarring. Crossfade deferred.

**Files changed:**
- `.claude/skills/user-test/scripts/ambient_background_exemplar.py` — DEFAULT_ALPHA=0.15, fixed log format, phase 3 pause 2s→5s

**Infrastructure built during this review:**
- gRPC server bind changed from `[::1]` to `0.0.0.0` (windowed.rs + headless.rs) — enables remote gRPC testing over Tailnet
- Python gRPC test client (`hud_grpc_client.py`) — session, lease, tile, node support via proto stubs
- Proto stubs generated in `proto_gen/` from `crates/tze_hud_protocol/proto/`

---

## 6. Dashboard Tile

**Type:** Tile | **Size:** 400x300 px

**Spec:** `openspec/changes/exemplar-dashboard-tile/`

### Design Review

- [ ] Tile size and proportions
- [ ] Dark background opacity
- [ ] Icon size and placement
- [ ] Markdown body rendering
- [ ] Button styling (Refresh/Dismiss)
- [ ] Button hit targets (touch-friendly?)
- [ ] Tile border/shadow treatment
- [ ] Orphan handling visual cue

### UX Tweaks

_(to be filled during review)_

---

## 7. Presence Card

**Type:** Tile | **Size:** 200x80 px | **Position:** bottom-left stack

**Spec:** `openspec/changes/exemplar-presence-card/`

### Design Review

- [ ] Card size — enough room for content?
- [ ] Avatar size and shape
- [ ] Agent name text size
- [ ] "Last active" timestamp readability
- [ ] Stack spacing when multiple agents
- [ ] Disconnect/offline visual treatment
- [ ] Card background contrast

### UX Tweaks

_(to be filled during review)_

---

## 8. Gauge Widget

**Type:** Widget | **Orientation:** vertical fill

**Spec:** `openspec/changes/exemplar-gauge-widget/`

### Design Review

- [ ] Fill bar animation smoothness (linear interp)
- [ ] Severity color mapping clarity
- [ ] Label text placement and size
- [ ] Background track contrast
- [ ] Widget dimensions
- [ ] Boundary behavior at 0% and 100%

### UX Tweaks

_(to be filled during review)_

---

## 9. Progress Bar

**Type:** Widget | **Size:** 300x20 px

**Spec:** `openspec/changes/exemplar-progress-bar/`

> **Note:** Asset bundle (widget.toml, track.svg, fill.svg) not yet created.

### Design Review

- [ ] Bar height — visible enough?
- [ ] Fill animation smoothness
- [ ] Centered label readability over fill
- [ ] Fill color customization
- [ ] Empty vs full visual distinction
- [ ] Corner radius treatment

### UX Tweaks

_(to be filled during review)_

---

## 10. Status Indicator

**Type:** Widget | **Size:** 48x48 px default

**Spec:** `openspec/changes/exemplar-status-indicator/`

### Design Review

- [ ] Circle size at default 48x48
- [ ] Color mapping clarity (online/away/busy/offline)
- [ ] Color-blind accessibility
- [ ] Optional label text placement
- [ ] Transition between states
- [ ] Visibility at small sizes

### UX Tweaks

_(to be filled during review)_

---

## Summary of Changes

_(to be filled after all reviews complete)_
