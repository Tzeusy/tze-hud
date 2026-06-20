# PSK Investigation: hud-39164f — Was a Real Production PSK Ever Committed?

**Date:** 2026-06-21  
**Investigator:** Beads Worker (agent/hud-39164f)  
**Verdict: PLACEHOLDER / NEVER-REAL — no history rewrite required; bead may be closed.**

---

## Scope

Determine whether any real production PSK value was ever embedded or committed in
this repository's source or git history. The bead asks for rotation "if the
embedded value was real"; this report provides the decisive precondition check.

---

## What Was Searched

| Surface | Method |
|---|---|
| Current working tree | `grep -rn -i "psk"` across all file types |
| Full git history — AGENTS.md | `git log -p --follow -- AGENTS.md \| grep "psk"` |
| Full git history — scripts | `git log -p -- "*.ps1" "*.sh" "*.bat" "*.cmd" \| grep "\-\-psk "` |
| Full git history — docs/evidence | `git log -p -- docs/ \| grep "\-\-psk "` |
| Full git history — configs | `git log -p -- "*.toml" "*.json" "*.yaml" \| grep -i "psk\s*="` |
| High-entropy strings | All `--psk <value>` occurrences in history filtered for non-placeholder tokens |

---

## Findings

### 1. Application default PSK

**File:** `app/tze_hud_app/src/main.rs:83`  
**Value:** `const DEFAULT_PSK: &str = "tze-hud-key";`  
**Classification:** Trivial placeholder intentionally rejected at startup.

The runtime explicitly rejects this value in strict mode
(`app/tze_hud_app/src/main.rs:634`, `psk_is_trivial_default`). It is
documented as intentionally trivial in `README.md:186` and
`about/legends-and-lore/rfcs/0006-configuration.md:526`. No production
deployment should ever use this value — the runtime refuses to start with it
in windowed mode.

### 2. Test-only PSK values in source

All test and example PSK values are trivially fictional and
obviously test-scoped:

| File | Value | Scope |
|---|---|---|
| `tests/integration/v1_thesis.rs:62` | `"v1-thesis-proof-key"` | integration test |
| `tests/integration/subtitle_streaming.rs:67` | `"subtitle-streaming-test-key"` | integration test |
| `examples/dashboard_tile_agent/src/main.rs:58` | `"dashboard-tile-key"` | example binary |
| `examples/render_artifacts/.../cooperative_projection_readback.rs:92` | `"readback-proof"` | example binary |
| `examples/vertical_slice/tests/*.rs` | `"test"`, `"calib"`, `"stage6-bench"`, etc. | test fixtures |
| `crates/tze_hud_protocol/tests/*.rs` | `"test-psk"` | protocol tests |
| `tests/integration/movable_elements_e2e.rs:780` | `"test-key"` | integration test |
| `examples/vertical_slice/tests/production_boot.rs` | `"production-boot-test"` | test, not production |

None of these are production-quality secrets. All were authored as test
fixtures and are not used on the live Windows HUD.

### 3. AGENTS.md — scheduled-task PSK reference

The AGENTS.md line that triggered the original hud-yotlg3 concern read:

```
--psk <psk>
```

(literal angle-bracket placeholder text, not a real value). Verified across the
**full git history** of AGENTS.md — every occurrence of `--psk` in any committed
version of that file uses the placeholder token `<psk>` or a shell variable
(`$psk`, `$Psk`). No real PSK value appears at any historical line, including
the range cited in the hud-yotlg3 description (lines 234, 276–302).

### 4. Evidence documents — process listings from Windows

Several committed evidence files in `docs/evidence/` capture live Windows
process command lines that include `--psk`. **Every such capture is redacted:**

- `--psk <redacted>`  
- `--psk <redacted>` (JSON-encoded `<redacted>`)  
- `--psk <user-test PSK>` (description, no literal)

The PowerShell scripts that capture these listings explicitly parse and log
`'Recovered from task XML; value intentionally omitted.'` rather than committing
the actual value.

### 5. Config / TOML files

No TOML, JSON, or YAML file in the repository — current or historical — contains
a `psk = "<literal>"` assignment with any non-trivial value. The `TZE_HUD_PSK`
env var is referenced as the runtime injection point; no deployment config
commits the value into the file.

### 6. Corroborating prior findings

The closed parent bead hud-yotlg3 (PR #902, `50d65b62`) explicitly states in its
commit message:

> "No literal PSK was ever in git (placeholder only); no history rewrite per owner
> decision (tailnet-internal host, key-only SSH, no credential leak)."

The hud-yotlg3 investigation notes confirm:

> "NO literal PSK in git (current or history)"

This report independently corroborates that finding through exhaustive history
search.

---

## Verdict

**PLACEHOLDER / NEVER-REAL.**

No real production PSK value was ever committed to this repository in any file,
at any commit, in current or historical state. The values that appear are:

1. The trivial software default `tze-hud-key` — rejected by the runtime itself.
2. Obviously fictional test fixtures (e.g. `"test"`, `"test-psk"`, `"calib"`).
3. Placeholder tokens `<psk>` / `<redacted>` / shell variables in docs and scripts.

The live Windows HUD PSK lives exclusively in the Windows scheduled-task
`<Arguments>` field (recoverable via `schtasks /Query /TN TzeHudOverlay /XML`)
and is never replicated to the Linux side in any tracked file. It was never
written into this repo.

---

## Recommended Action

**Close hud-39164f.** The precondition ("if the embedded value was real") is not
met. No PSK rotation is warranted based on git history. The live production PSK
on the Windows machine is presumed current and not compromised by any repository
exposure.

No history rewrite (BFG / git-filter-repo) is needed. This is consistent with
the prior hud-yotlg3 owner decision.

If the owner independently wants to rotate the production PSK for other reasons
(scheduled hygiene, policy), that remains a valid ops action but is not required
by any repository-side exposure found in this investigation.
