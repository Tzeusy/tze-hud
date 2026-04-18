# v2-embodied-media-presence — Procurement List

Owner: Tzeusy. This list records what must be acquired (hardware, SaaS, legal, libraries) to unblock each v2 phase. Pull-forward or defer as product priorities shift.

---

## Hardware

### Before phase 1 (gates real-decode CI per D18)

| Item | Purpose | Target cost | Notes |
|---|---|---|---|
| Self-hosted GPU runner box (RTX 4060-class or better) | Nightly real-decode CI, glass-to-glass latency measurement | ~$1,500 | Must sit on same LAN as existing CI; reliable power + cooling. Can be a repurposed workstation. |

### Before phase 3 (gates device lane per D19)

| Item | Purpose | Target cost | Notes |
|---|---|---|---|
| 1× latest iPhone | Primary iOS device lane | ~$1,000 | Current-gen preferred; older gen acceptable if budget-constrained. |
| 1× Pixel (latest) | Primary Android device lane | ~$800 | Pixel chosen for vanilla Android + quick security updates. |
| 1× MacBook (M-series) | Primary macOS desktop lane | ~$1,500 | Already owned if possible. |
| 1× Windows laptop | Primary Windows lane | — | Already in use per CLAUDE.md. Confirm model meets media-decode baseline. |
| 1× Linux laptop | Primary Linux lane | — | Probably already available. |

### Mid-phase 3 (glasses contract validation)

| Item | Purpose | Target cost | Notes |
|---|---|---|---|
| 1× XReal (Air 2 Pro or successor) | Constrained glasses profile: paired-desktop compositing | ~$500 | Entry-level AR glasses; tests upstream-composition path. |
| 1× Meta Ray-Ban | Constrained glasses profile: no-display embodied presence | ~$350 | Tests audio-only embodied presence + peripheral input. |
| 1× Apple Vision Pro | Capable glasses profile: full local compositing | ~$3,500 | Tests (a)-tier capability negotiation + standalone compositor. |

**Total glasses budget: ~$4,350.** Procure only once phase 3 begins exercising glasses in earnest — don't let hardware age in the drawer.

---

## SaaS / Cloud

### Phase 4b (cloud-relay sub-epic)

| Vendor | Purpose | Cost model | Selection notes |
|---|---|---|---|
| **LiveKit Cloud** (preferred) | WebRTC SFU for cloud-relayed media | Free dev tier; paid scales with minutes | Open-source server available for self-host post-v2. |
| Cloudflare Calls (alternate) | WebRTC SFU | Free up to threshold; paid per-minute | Less operator control than LiveKit; acceptable fallback. |

**Decision point**: commit to one vendor at phase 4b kickoff. Budget estimate: ≤$50/month during development, scales with demo traffic.

### External transcoding (deferred to v3)

Originally listed as v2 phase-4d. Project-direction audit demoted this to v3 — vendor choice was already deferred "until use case emerges", and leaving a deferred-vendor sub-epic in v2 invites partial preparation. V2 preserves transcoding abstraction points in the `media-plane` spec; no v2 procurement required. Candidates for v3 if ever needed: AWS MediaConvert, Cloudflare Stream, Mux.

---

## Legal / professional services

### Before phase 4a (recording ships to users)

| Service | Purpose | Target cost | Timing |
|---|---|---|---|
| Tech-sector privacy counsel review (2–4 hr) | Review recording consent UI, retention defaults, jurisdiction-detection behavior for 2-party-consent states (CA, WA, etc.), EU GDPR, Australian Surveillance Devices Act | ~$1,500 | Before phase 4a ships to any user. |

### Federation (deferred to v3)

Originally listed as v2 phase-4c with a follow-up privacy counsel re-review (~$750). Project-direction audit demoted federation to v3 — cross-operator policy merge is scope inflation vs. the "one household's screens" thesis. V2 reserves federation-aware *fields* in data structures but ships no 4c sub-epic; no v2 counsel review required for federation. When federation is taken up in v3, budget ~$750 for the re-review and use the same counsel engaged for 4a to reduce scope drift.

---

## Library audits

Each library audit is a bead under its corresponding phase. Audit completes before any implementation bead depending on the library opens.

| Library | Phase | Audit scope |
|---|---|---|
| `webrtc-rs` | 1 | Maturity, security posture, maintainer activity, API stability, decode codec support coverage |
| `gstreamer` + gstreamer-rs | 1 | Already locked in per CLAUDE.md; audit is for version pinning + feature flag coverage |
| `statig` or chosen state machine crate | 2 | API fit for embodied/media state machines, macro hygiene, no-std support if glasses ever needs it |
| Chosen SFU signaling shim | 4b | Depends on C15 vendor selection — LiveKit SDK or Cloudflare Calls SDK. Audit for Rust bindings maturity, secret handling, telemetry leakage |
| `cpal` or chosen audio I/O crate | 1 | Runtime-owned audio routing per E22. Platform coverage (WASAPI/CoreAudio/ALSA/PipeWire) |

---

## Budget summary (planning estimate)

| Category | Pre-phase-1 | Phase-3 | Phase-4 | Total |
|---|---|---|---|---|
| Hardware (GPU runner) | ~$1,500 | — | — | ~$1,500 |
| Hardware (primary devices) | — | ~$3,300 | — | ~$3,300 |
| Hardware (glasses) | — | ~$4,350 | — | ~$4,350 |
| SaaS (cloud-relay, dev-scale) | — | — | ~$500/yr | ~$500/yr |
| Legal (privacy counsel, v2 only) | — | — | ~$1,500 | ~$1,500 |
| **Total one-time hardware + legal** | | | | **~$10,650** |
| **Recurring SaaS** | | | | **~$500/yr** |

All figures are rough planning estimates; treat as ±30%.

---

## Procurement order (recommended)

Match procurement to phase gates so nothing ages in the drawer:

1. **Now (pre-v1 ship)**: GPU runner box. Blocks phase 1 real-decode CI.
2. **At phase 3 kickoff**: iPhone + Pixel + any missing laptops. Blocks device lane on mobile.
3. **At phase 3 mid-point**: XReal + Ray-Ban Meta (cheaper SKUs exercised first).
4. **At phase 3 late / phase 4 early**: AVP if the capable-glasses lane is in active use.
5. **Phase 4a kickoff**: privacy counsel review scheduled (recording-only, v2).
6. **Phase 4b kickoff**: SaaS SFU vendor signup + audit.
