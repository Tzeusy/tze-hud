# Dependency Upgrade Ledger

Consolidated tracking of pinned dependencies whose upgrade is deliberately
deferred, the blockers, and the expected impact. The authoritative constraints
live in `Cargo.toml` and `deny.toml`; this ledger aggregates them so the upgrade
runway is legible in one place.

## Current floor

- **MSRV:** Rust `1.88` (`Cargo.toml` `[workspace.package].rust-version`, pinned
  in `rust-toolchain.toml`).
- **Render/text stack co-pin:** `wgpu = "24"`, `glyphon = "0.8"`
  (`Cargo.toml` `[workspace.dependencies]`).

## Tracked upgrade: glyphon / wgpu / MSRV chain

| Field | Value |
|---|---|
| Pinned | `glyphon 0.8.x` → `wgpu 24.x` → Rust `1.88` |
| Target | `glyphon 0.9+` → `wgpu 28+` → Rust `1.92+` |
| Blocker | `glyphon 0.9+` requires `wgpu 28+`, which requires Rust `1.92+` MSRV |
| Source of truth | `Cargo.toml` lines ~33–55 (`rust-version`, `wgpu`, `glyphon` notes) |
| Expected impact | Render/compositor crates recompile against new `wgpu` surface/texture APIs; GPU pixel-readback (`test-gpu-pixel-readback`) and Windows performance-budget lanes are the highest-risk re-validation surfaces. Bump `rust-version` and `glyphon` together. |

## Tracked advisory: RUSTSEC-2024-0436 (`paste` unmaintained)

| Field | Value |
|---|---|
| Advisory | `RUSTSEC-2024-0436` — `paste` is unmaintained (archived by dtolnay) |
| Chain | `paste ← metal ← wgpu-hal ← wgpu ← glyphon` (transitive) |
| Risk | Compile-time proc-macro only; no runtime attack surface |
| Disposition | Allowed in `deny.toml` `[advisories]` |
| Removal condition | Resolves automatically when the glyphon/wgpu chain above lands (`wgpu 28+`, Rust `1.92+`) |

## Maintenance

When the glyphon/wgpu/MSRV chain is upgraded, remove the `RUSTSEC-2024-0436`
allow from `deny.toml`, update the co-pin notes in `Cargo.toml`, and update this
ledger. Add a new row here for any future deliberately-deferred dependency.
