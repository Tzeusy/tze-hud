# Windowed Decomposition Reconciliation

Issue: `hud-e1r18v`
Scope: documentation/spec reconciliation after PR #934 (`03b6e66f`) completed the
windowed runtime facade slim-down.

## Evidence

- The former hotspot file `crates/tze_hud_runtime/src/windowed.rs` no longer
  exists at head.
- The current source is `crates/tze_hud_runtime/src/windowed/`, with no file
  above the ~3,000-line hotspot threshold:
  - `mod.rs`: 1,873 lines
  - `config.rs`: 412 lines
  - `network.rs`: 499 lines
  - `hittest.rs`: 545 lines
  - `input_dispatch.rs`: 1,215 lines
  - `keyboard.rs`: 905 lines
  - `lifecycle.rs`: 1,684 lines
  - `portal.rs`: 2,849 lines
  - `widgets.rs`: 465 lines
  - `test_support.rs`: 1,604 lines
- The closing PR tranche is merged on `main`:
  - #920 `refactor: extract windowed config [hud-cowvzn]`
  - #922 `refactor: extract windowed network helpers [hud-4a0vyr]`
  - #930 `refactor: extract windowed portal module [hud-nhi87f]`
  - #931 `refactor: extract windowed lifecycle module [hud-sp5hsg]`
  - #934 `refactor: slim windowed facade [hud-xmo18w]`

## Reconciliation Result

The decomposition is behavior-preserving from the documentation/spec standpoint:
the public runtime entry remains `tze_hud_runtime::windowed::WindowedRuntime`
with `WindowedConfig` and `WindowedBenchmarkConfig` re-exported by
`windowed/mod.rs`, while subsystem implementation moved behind private module
seams. No capability contract needs a behavioral delta for this refactor.

Active OpenSpec pointers that cited the removed `windowed.rs` source path were
updated to the new module locations. The topology map now records the split
module responsibilities and line counts, and the engineering-bar hotspot ledger
marks the original `windowed.rs` hotspot as closed.

No behavior/spec drift was introduced by the decomposition.
