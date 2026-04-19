# Safari macOS CI Runner — Simulcast Interop Lane Design

**Issue:** `hud-clwaj`
**Date:** 2026-04-19
**Author:** agent worker (claude-sonnet-4-6)
**Parent plan:** `docs/testing/simulcast-interop-plan.md` (hud-fpq51, PR #538)
**Companion stub:** `.github/workflows/safari-simulcast-interop.yml`
**Status:** Design document — harness not yet implemented

---

## 1. Why a Safari Runner Is Required

### 1.1 Safari's WebRTC implementation

Safari's WebRTC stack is a divergent fork of Google's libwebrtc maintained by the
WebKit team. Unlike Chrome (which tracks upstream libwebrtc closely) and Firefox
(which has its own SIPCC/Neko implementation), Safari's WebKit fork:

- Is updated on Safari's release cadence (slower than Chrome's 6-week cycle).
- Has historically lagged in simulcast support, particularly for VP9 simulcast.
- Uses a separate SDP negotiation path with known quirks around RID/MID handling
  in older versions.
- Does not expose certain WebRTC internals via `RTCInboundRtpStreamStats` in the
  same way as Chrome, requiring Safari-specific assertion strategies.
- Requires a real macOS host — Safari cannot run in a Linux container or Windows VM.

### 1.2 Simulcast quirks specific to Safari

As documented in the simulcast interop plan (§3.3 of hud-fpq51):

- **VP9 simulcast**: Not guaranteed across all Safari versions. Safari 16.4 added
  VP9 support, but simulcast with VP9 is experimental and may require explicit SDP
  manipulation.
- **H.264 simulcast**: Generally functional via the hardware encoder path, but the
  RID extension parsing differs from Chrome/Firefox behavior in Unified Plan.
- **Plan-B vs Unified Plan**: Safari 15+ uses Unified Plan by default; Safari 14 and
  earlier use Plan-B. tze_hud must emit Unified Plan SDP with `a=rid` and
  `a=simulcast` lines (RFC 8853). Always negotiate with Safari 15+ minimum.
- **BUNDLE/MID**: Safari's BUNDLE extension handling is correct in recent versions
  (16+) but has had interop issues with some SDP flavors.

### 1.3 Why a separate CI lane is needed

The existing CI runs exclusively on `ubuntu-latest` runners. Safari:

- Cannot be installed on Linux.
- Requires macOS to run at all (safaridriver is macOS-only; iOS simulation requires
  Xcode, also macOS-only).
- Has no headless mode in the Docker-compatible sense — `safaridriver` can run with
  the display suppressed but the full macOS user session must exist.

Safari is therefore categorically excluded from the Linux CI matrix regardless of
containerization strategy. A dedicated macOS GitHub Actions runner is mandatory.

### 1.4 Connection to the browser × codec matrix (hud-fpq51 §2 + §3)

The simulcast interop plan (hud-fpq51, PR #538) defines the browser × codec matrix
in §3.2. Safari cells are CONDITIONAL (not MUST PASS) for both H.264 and VP9
simulcast, meaning:

- Failure does not block Phase 4b ship but requires a named tech-lead waiver.
- The Safari CI lane must run to produce results — a missing result is treated the
  same as a failure for gate purposes.

The macOS runner is therefore not optional: if the lane does not exist, the
CONDITIONAL cells have no result and Phase 4b closeout cannot record them.

---

## 2. GitHub Actions Runner Pinning — `macos-14` vs `macos-latest`

### 2.1 Available macOS runners

As of 2026, GitHub Actions offers three macOS runner tiers:

| Runner label | Architecture | Notes |
|---|---|---|
| `macos-12` | Intel x86_64 | Older, will be deprecated |
| `macos-13` | Intel x86_64 | Current Intel option |
| `macos-14` | Apple Silicon (M1/M2) | ARM64; faster; newer default |
| `macos-latest` | Currently `macos-14` | Floating alias — will advance |

### 2.2 Pricing implications

GitHub Actions bills macOS runners at approximately **10× the per-minute rate of
Linux runners** (as of the GitHub Actions billing schedule, April 2026):

| Runner | Billable rate (approx) | Relative cost |
|---|---|---|
| `ubuntu-latest` | 1× | Baseline |
| `macos-14` | ~10× | ~10× per minute |
| macOS M1 (self-hosted) | Hardware + maintenance | Variable |

A single Safari interop run that takes 20 minutes would cost approximately the
same as 200 minutes of Linux time. **This is why the lane must not fire on every
PR.**

### 2.3 Runner recommendation: `macos-14` (pinned)

**Recommendation: pin to `macos-14` rather than `macos-latest`.**

Rationale:

1. **Stability over convenience**: `macos-latest` is a floating alias. When GitHub
   advances it from `macos-14` to `macos-15`, the Xcode version and Safari version
   change without a PR touching the workflow file. Simulcast assertions are
   Safari-version-sensitive; an unexpected Safari upgrade could silently change test
   results.

2. **Architecture consistency**: `macos-14` is Apple Silicon (M1). This is the
   correct architecture for validating Safari on macOS since the M-series transition.
   macOS 12/13 (Intel) are legacy and being sunset.

3. **Safari version pinning**: On a pinned `macos-14` runner image, the Safari
   version is determined by the macOS image version, which GitHub updates on a
   defined schedule and documents in release notes. This makes regressions traceable
   to an image bump.

4. **Migration path**: When `macos-15` is required (e.g., for Safari 18+ features),
   an explicit PR updates the pinned runner label. This creates a visible decision
   point with a commit trail.

**How to verify the Safari version on a given runner image:**

```yaml
- name: Report Safari version
  run: /usr/bin/safaridriver --version
```

At phase 4 kickoff, record the Safari version in the interop test result and
verify it matches the target range (16.4+ per hud-fpq51 §3.1).

### 2.4 iOS Safari (WKWebView)

The simulcast interop plan mentions Safari (iOS) 16.4+ as a target (§3.1). iOS
Safari testing requires the iOS Simulator, which is only available on macOS with
Xcode installed. The `macos-14` GitHub Actions runner includes Xcode and can run
`xcrun simctl` to launch an iOS Simulator.

**Recommendation**: Scope the initial lane to macOS Safari only. iOS Simulator
tests are additive and can be implemented once the macOS lane is stable. They
should run on the same `macos-14` runner to avoid a separate machine allocation.

---

## 3. Safari Technology Preview vs Shipping Safari

### 3.1 Shipping Safari (primary)

The primary target is the shipping Safari version present on the runner image
(currently Safari 16.x–17.x on `macos-14`). This is the version actual operators
and end-users run. All interop assertions must pass against shipping Safari.

### 3.2 Safari Technology Preview (secondary, informational)

Safari Technology Preview (STP) is Apple's experimental release channel. STP often
ships WebRTC improvements 6–12 months before they land in shipping Safari. Testing
against STP serves two purposes:

1. **Early warning**: A failing STP result that passes in shipping Safari signals
   a regression that will arrive in production within months.
2. **Feature availability**: STP sometimes enables VP9 simulcast or RID features
   before they are stable. STP results annotate CONDITIONAL cells with "passes in
   STP but not shipping."

**Recommendation**: Run STP as a separate matrix row in the same workflow, gated
as informational (non-blocking). Install STP via:

```bash
# Install Safari Technology Preview via Homebrew
brew install --cask safari-technology-preview
```

STP installs as a separate application (`/Applications/Safari Technology Preview.app`)
and has its own safaridriver binary. The workflow must explicitly point to it.

**Do not substitute STP for shipping Safari**: gate results must come from the
shipping version. STP results are recorded alongside but do not count for go/no-go
gate purposes.

---

## 4. Browser Automation Approach

### 4.1 safaridriver (WebDriver)

`safaridriver` is Apple's WebDriver implementation for Safari. It ships with macOS
and Xcode and requires no additional installation on GitHub-hosted macOS runners.
Enable it with:

```bash
sudo safaridriver --enable
```

`safaridriver` implements the W3C WebDriver protocol, not WebDriver BiDi. It does
not support Chrome DevTools Protocol (CDP). This limits what can be observed from
JavaScript running inside the browser:

- Supported: `RTCPeerConnection` creation, SDP exchange, JS injection via
  `executeScript`.
- Supported: Reading `RTCInboundRtpStreamStats` via `getStats()`.
- Not supported: CDP-level network interception.

### 4.2 WebDriver BiDi

WebDriver BiDi is the successor to WebDriver and CDP. Safari 17+ has partial BiDi
support, but it is not stable enough for production CI as of April 2026. Do not
depend on BiDi for Safari interop assertions. Re-evaluate at Phase 4 kickoff.

### 4.3 Playwright and the hud-mc3ka concern

The simulcast interop plan (§4.4) references Playwright as the browser automation
layer. Playwright supports Safari/WebKit via its `webkit` engine. However, as
documented in **hud-mc3ka** (open), the plan's §5.3 feature flag incorrectly
specifies `dep:playwright` — Playwright is a Node.js/Python library with no native
Rust crate.

Two viable approaches for the Rust harness:

#### 4.3.1 Approach A: Playwright as external subprocess (recommended)

Run Playwright as a Node.js subprocess managed by the Rust test harness:

```
Rust test binary
  → spawns: node playwright-bridge.js
  → communicates via: stdin/stdout JSON-RPC or local TCP
  → Playwright controls: Safari WebKit engine
  → assertions: JS injected via Playwright page.evaluate()
```

This is the approach most aligned with the existing plan's intent. The Playwright
bridge script is a thin adapter (~100 LOC) that accepts commands from the Rust
harness and returns `RTCInboundRtpStreamStats` results.

**Feature flag fix (per hud-mc3ka)**: Replace `dep:playwright` in the Cargo
feature flag with a marker feature that has no crate dependency:

```toml
[features]
simulcast-interop = ["dep:webrtc"]
simulcast-interop-playwright = ["simulcast-interop"]  # no Rust crate dep
```

The presence of `simulcast-interop-playwright` signals to CI that the Node.js
Playwright bridge should be set up in the pre-step.

#### 4.3.2 Approach B: chromiumoxide / WebDriver Rust crate

Use a Rust-native browser automation crate:

- `chromiumoxide` (crates.io): CDP-based Chrome/Chromium automation. Does not
  support Safari.
- `webdriver` (crates.io): Low-level W3C WebDriver client. Supports safaridriver
  but requires more boilerplate.

For Safari specifically, a raw `webdriver` crate talking to `safaridriver` is
feasible. This avoids the Node.js subprocess but provides a less ergonomic API for
JS injection and stat collection.

**Recommendation**: Use Playwright subprocess (Approach A) for the harness
implementation. It provides the richest API for stat collection and is the
approach already assumed by the existing plan. hud-mc3ka tracks the required
Cargo feature flag correction.

### 4.4 Safari-specific setup steps (CI pre-flight)

Before the test run, the CI job must:

```bash
# Enable safaridriver (requires sudo on GitHub-hosted runners)
sudo safaridriver --enable

# Verify safaridriver responds
safaridriver --version

# Install Playwright (if using Approach A)
npm install -g playwright
npx playwright install webkit

# Verify webkit browser binary is installed
npx playwright install-deps webkit
```

Note: `npx playwright install webkit` downloads Apple's WebKit build maintained
by the Playwright project. This is distinct from the system Safari — it is a
headless-capable WebKit build. For testing against **system Safari**, use
`safaridriver` directly, not Playwright's WebKit build.

**For real Safari (system) testing**: The Playwright `page.goto()` API with
`browserType: 'webkit'` uses Playwright's WebKit build, not system Safari. To
test against the actual Safari.app (required for accurate simulcast interop
results), the harness must use `safaridriver` directly or use Playwright's
`browserType.connectOverCDP()` to connect to a safaridriver session.

This distinction is important: **Playwright's WebKit ≠ Safari**. The browser
× codec matrix cells require testing against system Safari to produce valid gate
results.

---

## 5. Runner Gating Strategy

### 5.1 Trigger matrix

The Safari interop lane should fire on three triggers, never on standard PR push:

| Trigger | When | Rationale |
|---|---|---|
| `workflow_dispatch` only | Manual: developer explicitly requests it | Zero accidental cost; full control |
| Nightly schedule | `cron: '0 3 * * *'` (3am UTC) | Regular validation without blocking PR flow |
| Label trigger | PR label `run-safari-interop` | Explicit opt-in for PRs that touch WebRTC paths |

**Do NOT add `push:` or `pull_request:` triggers.** A push-triggered macOS run
costs ~10× Linux; on a busy repo this would consume the billing budget in days.

### 5.2 Label-gated trigger pattern

GitHub Actions does not support label filtering in the `on:` block directly for
`pull_request` events (filtering by label requires a job-level `if:` condition):

```yaml
on:
  pull_request:
    types: [labeled]

jobs:
  safari-interop:
    if: github.event.label.name == 'run-safari-interop'
    runs-on: macos-14
```

Alternatively, use the `pull_request` event with an `if:` check on
`contains(github.event.pull_request.labels.*.name, 'run-safari-interop')` — this
fires on every PR update but exits immediately if the label is absent, incurring
only scheduling overhead.

### 5.3 Nightly schedule

A nightly run (3am UTC) provides a regular health signal without burning PR quota:

```yaml
on:
  schedule:
    - cron: '0 3 * * *'  # 3am UTC nightly
```

The nightly run should target the `main` branch. If `main` lacks the harness crate
(pre-Phase 4), the nightly run should be disabled (not just no-op) to avoid
paying for a macOS allocation that does nothing. The stub workflow (§8) is gated
on `workflow_dispatch` only and has no `schedule:` trigger for exactly this reason.

### 5.4 Recommended label for PR opt-in

Create a GitHub repository label:

- Name: `run-safari-interop`
- Color: `#E87722` (orange — expensive run)
- Description: "Trigger Safari macOS interop CI lane (expensive, use deliberately)"

---

## 6. Cost Estimate Per Run

### 6.1 Assumptions

- GitHub-hosted `macos-14` runner: ~10× per-minute rate vs `ubuntu-latest`.
- `ubuntu-latest` billing rate: ~$0.008 per minute (GitHub Actions billing as of
  2026).
- `macos-14` billing rate: ~$0.08 per minute.

### 6.2 Estimated run duration

| Phase | Estimated time | Notes |
|---|---|---|
| Runner provisioning | 3–5 min | macOS images are larger; slower to start |
| Actions checkout + cache restore | 1–2 min | Rust cache on macOS is slower |
| Rust build (harness crate) | 10–20 min | First build; cache warms subsequent runs |
| Node.js + Playwright install | 2–3 min | Playwright WebKit download ~100 MB |
| safaridriver enable + verify | < 1 min | |
| Test execution (per browser × codec cell) | 3–5 min per cell | 6 cells (H.264 + VP9 × 3 layers + layer switch) |
| Total (warm cache, 6 cells) | ~25–30 min | Cold build: ~40–50 min |

### 6.3 Cost per run

| Scenario | Duration | Estimated cost |
|---|---|---|
| Warm cache, 6 cells | 25 min | ~$2.00 |
| Cold build, 6 cells | 45 min | ~$3.60 |
| Full matrix (macOS + iOS Simulator) | 60 min | ~$4.80 |

These are rough estimates. Actual cost depends on GitHub's billing schedule and
whether the repo is on a paid plan or free tier. **Budget ~$2–5 per run and
design the run frequency accordingly** (nightly = ~$60–150/month).

### 6.4 Cost mitigation

- Cache the Rust build artifacts aggressively (`Swatinem/rust-cache`).
- Cache the Playwright WebKit binary separately (changes rarely).
- Run only the Safari-relevant cells (H.264 and VP9 simulcast; skip cells that are
  Linux-only).
- Skip the test run if no changes to `crates/tze_hud_webrtc_interop/` are detected
  (for label-triggered runs, check changed files before provisioning macOS).

---

## 7. Prerequisites Before This Lane Can Activate

The stub workflow exists in the repo before these prerequisites are met, but it
must not have `schedule:` or `push:` triggers until all items below are complete.
Enable the nightly trigger only when the harness is ready to produce valid results.

### 7.1 Required prerequisites

| # | Prerequisite | Tracking bead | Status |
|---|---|---|---|
| 1 | `crates/tze_hud_webrtc_interop/` crate exists in the workspace | hud-fpq51 Phase 4 implementation bead | Pending (Phase 4) |
| 2 | Simulcast publisher trait implemented (`SimulcastPublisher`) | hud-fpq51 Phase 4 | Pending |
| 3 | Safari-specific test file `tests/simulcast_safari.rs` authored | hud-fpq51 Phase 4 | Pending |
| 4 | Local WebSocket signaling server implemented | hud-fpq51 Phase 4 | Pending |
| 5 | Playwright subprocess bridge (`playwright-bridge.js`) authored | hud-mc3ka + Phase 4 | Pending |
| 6 | Cargo feature flag fix (dep:playwright → subprocess marker) | hud-mc3ka | Open |
| 7 | WebRTC transport layer selected (webrtc-rs v0.20 or str0m) | hud-g89zs Phase 4 kickoff | Pending |
| 8 | SFU adapter seam validated (LiveKit or Cloudflare Calls) | hud-s2j0l | In flight |
| 9 | GitHub repository label `run-safari-interop` created | — | Manual step |
| 10 | Safari version on target runner image documented | — | Phase 4 kickoff |

### 7.2 Activation checklist

When all prerequisites above are complete, remove the activation gate by:

1. Adding the `schedule:` trigger to the workflow file.
2. Adding the `pull_request` label trigger.
3. Removing the `NOT ACTIVE` comment header.
4. Recording the Safari version under test in the workflow or a companion doc.
5. Creating a Phase 4 bead to run the first full matrix execution and record results.

---

## 8. Stub Workflow

See companion file: `.github/workflows/safari-simulcast-interop.yml`

The stub is a fully valid YAML file gated on `workflow_dispatch` only (no
`schedule:` or `push:` triggers). It will not fire accidentally. The stub:

- Uses `macos-14` (pinned, not `macos-latest`).
- Includes placeholder steps that document the intended flow.
- Passes YAML validation (`python3 -c "import yaml; yaml.safe_load(...)"` must pass).
- Has a comment header stating it is not active pending hud-clwaj prerequisites.

---

## 9. Cross-References

| Reference | Relevance |
|---|---|
| `docs/testing/simulcast-interop-plan.md` | Source plan; §3.3 Safari constraints; §5.2 CI placement |
| `.github/workflows/ci.yml` | Existing CI conventions to follow |
| `.github/workflows/android-bootstrap.yml` | Precedent for platform-specific workflow structure |
| `docs/ci/android-gstreamer-bootstrap.md` | Precedent for CI design doc format |
| hud-fpq51 | Phase 4 simulcast interop plan (PR #538, merged) |
| hud-mc3ka | Playwright Cargo feature flag fix (open) |
| hud-s2j0l | SFU adapter seam (in flight) |
| hud-g89zs | webrtc-rs v0.20 simulcast spike (NO-GO verdict) |
| hud-1ee3a | str0m fallback audit (PR #544) |

---

*End of document. Activate nightly trigger after all prerequisites in §7 are met.*
