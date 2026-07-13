# Efficiency

> Adopted 2026-07-13 by owner mandate. This file makes explicit what was
> previously implied and scattered: tze_hud is a **highly compute- and
> token-efficient HUD runtime**, and that efficiency is not an optimization
> pass — it is a survival requirement for where this runtime is going.

## The deployment trajectory

The runtime has one architecture and an expanding set of power envelopes:

1. **Today: desktop PC overlays.** The active shipping target — a transparent,
   always-on-top, click-through Windows overlay (see `v1.md`). Desktop has
   compute headroom; we deliberately refuse to spend it.
2. **Eventually: smart glasses and VR headsets.** The declared endgame is this
   runtime operating as the HUD layer of glasses- and headset-class devices:
   battery-powered, thermally throttled, memory-starved, with unforgiving
   display duty cycles — and, for VR, stereo presentation at 90–120Hz where
   missed frames are physically felt.

Execution remains single-Windows — Windows-only, per `v1.md` §Single-Windows
Refocus, which governs what is built now (`mobile.md` keeps device-specific
implementation parked). What
this file changes is the **design pressure**: every decision made on the
desktop runtime today must survive the wearable envelope tomorrow. Desktop
headroom is a test environment, not a budget.

## Two currencies, two budgets

The runtime spends two scarce currencies, and doctrine treats both as
product-defining:

### Compute budget

Frames, watts, and memory. The compositor's cost model must satisfy:

- **Idle screens cost nothing.** A scene with no changes and no animations
  presents nothing — no per-frame GPU submission, approximately zero CPU.
  Idle cost is a measured property, not an assumption — a HUD that drains a
  battery while showing a static glanceable is defective.
- **Work is proportional to change.** One dirty subtitle re-renders one
  subtitle. Full-scene re-composition in response to a one-node diff is an
  anti-pattern wherever the platform allows better.
- **Degradation is designed, not discovered.** The degradation ladder
  (`failure.md`) and capability negotiation (`mobile.md`) exist so the same
  scene model runs honestly at a smaller budget. Features that only work with
  desktop-class headroom must degrade or be rejected at design time.
- **Budgets are enforced on constrained hardware.** Performance claims must
  be validated against low-power envelopes (software rasterizers, small-core
  CPUs), not only against developer workstations — a constrained-envelope
  lane of `validation.md`'s hardware-calibration vector. The reference
  numbers in `v1.md` are ceilings measured where they are cheapest to hit;
  the doctrine target is the envelope where they are hardest.

### Token budget

The runtime's primary clients are LLMs, and LLMs pay for every byte that
crosses their context. The API surface is therefore designed for **metered
intelligences**:

- **The runtime does the design; the model states intent.** Layout, geometry,
  styling, chrome, and animation live server-side (zones, design tokens,
  component profiles, rendering policies). A model publishes semantic content
  in a handful of small calls; pixels, positions, and polish never pass
  through model context. Bound typed semantic parameters against server-side
  templates (a widget's `color`/`enum`/`f32` value per `v1.md`'s widget
  contract) are intent, not design, and remain permitted; what is forbidden
  is raw layout, geometry, pixel positioning, and full-styling payloads. If
  driving a surface requires the model to emit those, that surface is
  misdesigned.
- **Deterministic scripts over model improvisation.** Every recurring
  operational flow (deploy, attach, publish, poll, verify) is captured as a
  deterministic script or tool the model invokes with a few tokens, rather
  than a procedure the model re-derives each session. Discovery cost is paid
  once, then encoded.
- **Append, coalesce, long-poll.** Publishing appends fragments — never
  re-sends transcripts. Streams coalesce with latest-wins keys. Awaiting
  input is one long-poll, not a busy loop. State the model does not need is
  not returned to it.
- **Token cost is a product metric.** Canonical flows (publish a zone
  message, hold a text-stream portal conversation, run a status dashboard)
  must have measured token footprints, tracked alongside latency budgets. A
  regression that doubles the tokens needed to hold presence is a
  performance regression.

The two budgets reinforce each other: server-side design is what makes the
model's footprint small, and semantic (rather than pixel-pushing) protocols
are what keep the wire and the compositor cheap.

## Why this is doctrine and not tuning

A dashboard can afford to be wasteful; a presence engine cannot. Presence
means being on screen for hours — on a wall display, over a desktop, inside
glasses. At that duty cycle, inefficiency compounds into heat, battery drain,
fan noise, dropped frames, and API bills: all of them presence-destroying.
The product thesis (`vision.md` §Performance is part of the product) already
holds that performance is meaning; this file extends the same claim to watts
and tokens.

## Anti-patterns

- Re-rendering or re-compositing unchanged content every frame.
- Any design that assumes desktop-class GPU/CPU headroom without a stated
  degradation path.
- Verbose JSON or full-state snapshots on hot paths (re-affirming
  `architecture.md`).
- LLMs in the frame loop, or LLM polling loops standing in for event
  delivery (re-affirming the prime rule).
- APIs that require models to send or receive layout/styling/geometry data.
- Recurring operational procedures that live only in a model's context
  instead of a deterministic script.
- Treating token footprint as free because a session happens to have budget.

## Relationship to other doctrine

- `vision.md` — performance-is-product; this file adds the efficiency and
  deployment-trajectory dimensions.
- `mobile.md` — the deferred device profiles this trajectory eventually
  lands on; implementation stays parked, envelope pressure applies now.
- `architecture.md` — message classes and protocol planes are the wire-level
  mechanics this doctrine's budgets constrain.
- `attention.md` — the attention budget is the human-side analogue: screens
  and humans are both finite resources the runtime governs.
- `validation.md` — where efficiency budgets must live as measured,
  hardware-normalized, CI-enforced properties.
