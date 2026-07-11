# Live bridge-routing verification — `TZE_HUD_RESIDENT_GRPC_PORTAL` end-to-end

**Bead:** hud-rw8eo (live-Windows verification of hud-hfuxy / PR #1046)
**Date:** 2026-07-11
**Host:** autonomous `windows-vm.example` HUD testhost (Proxmox `proxmox-host.example`, software GPU / WARP-Vulkan), scene 1280×800, gRPC 50051 / MCP 9090.
**Binary:** `tze_hud.exe` built fresh from `origin/main` `cae4b302`, `x86_64-pc-windows-gnu` release —
`sha256 7994d9843c07be68127665fe939ba34a9defb7d53792a6e4fd2b08d2ee906ca9` (see `logs/exe.sha256`).
Predecessor deployed exe (`af6b215c`) predated today's merges; this run is on a current-main build.

## What was verified

hud-g7ool (#1028) added the `PortalTransport` per-projection discriminant; hud-hfuxy (#1046)
wires `dispatch_portal_op`'s `Attach` handler to route newly-attached projections onto
`PortalTransport::ResidentGrpcBridge` **whenever the resident gRPC bridge channel is installed**
(`--resident-grpc-portal` / `TZE_HUD_RESIDENT_GRPC_PORTAL=1`), suppressing their in-process
direct-scene arm. This run exercises that wiring **live** over the real bridged transport.

## Enabled-bridge proof

The runtime was launched with `--resident-grpc-portal` against the canonical config plus a
`[agents.registered.resident-grpc-portal]` grant (required, else the bridge handshake fails
`CapabilityNotGranted`). Runtime startup log (`logs/bridge-startup.log`, captured via a
stdout-redirected launch) shows the bridge **enabling and connecting**:

```
INFO tze_hud_runtime::windowed: resident gRPC portal bridge enabled (two adapter families; hud-d7frs) endpoint=http://127.0.0.1:50051 agent_id=resident-grpc-portal lease_ttl_ms=60000
INFO tze_hud_runtime::resident_grpc_bridge: resident gRPC portal bridge connected (two-adapter-families gate)
```

(That particular capture launched via a `cmd.exe` wrapper for stdout redirection, which runs
non-interactively and cannot create a Vulkan surface — hence the trailing compositor panic in
that log. It is a launch-context artifact, NOT a bridge fault; the actual evidence run below
used the interactive exe-direct launch, which renders, and proves routing over gRPC.)

## Routing proof — the tile-owner namespace (A/B control)

There is no `transport` field on the wire (WM-S2b snapshot exclusion) and runtime tracing is
stdout-only. Routing is proven instead by **who owns the materialised portal tile**, read from a
throwaway gRPC observer's `SceneSnapshot` (`tiles[hostTileId].namespace`). The bridge creates the
tile from its own loopback `HudSession` as agent `resident-grpc-portal`; the in-process driver
paints under `tze_hud_portal_driver`. The in-process path can never produce a
`resident-grpc-portal` tile. Same MCP attach path + same observer; only the bridge flag differs:

| Runtime config | portal tile `namespace` | Materialiser | Snapshot |
|---|---|---|---|
| **bridge OFF** (default `tze_hud.toml`, no flag) | `tze_hud_portal_driver` | in-process direct-scene | `snapshots/00-control-in-process.json` |
| **bridge ON** (`--resident-grpc-portal`) | `resident-grpc-portal` | resident-gRPC bridge | `snapshots/01-bridged-baseline.json` |

The owner flips exactly as hud-hfuxy specifies.

## Per-check verdicts

The observer's `SceneSnapshot` also carries the bridged portal's **rendered transcript
markdown** at `nodes[].data.TextMarkdown.content` — so the actual streamed units and the
rendered unread indicator are observable over gRPC, not merely the part topology.

| # | Check | Verdict | Evidence |
|---|-------|---------|----------|
| 1 | Portal **attach + publish renders via the bridge** (not the in-process arm) | **PASS** | Bridged tile `namespace = resident-grpc-portal` vs control `tze_hud_portal_driver`; unit A present in the bridged tile's rendered `TextMarkdown` content. `snapshots/00`,`01`; `logs/verdicts.json §1`. |
| 2 | **Transcript streaming** updates flow end-to-end over the bridge | **PASS** | 4 sequential `portal_projection_publish` (A–D) — **all four units A,B,C,D appear in the bridged portal's rendered `nodes[].data.TextMarkdown.content`** (baseline showed only A), surface `lifecycle=Active`, tile stays `resident-grpc-portal`. Proves the bridge keeps applying streamed updates, not just that a Transcript part exists. `snapshots/01`→`02`; `§2`. |
| 3 | **Unread count / jump-to-latest** parity on the bridged path (#1107) | **PASS (rendered-indicator parity)** | The unread indicator (`"N unread"`) renders in the bridged portal's `TextMarkdown` content and **matches the in-process control's rendered indicator** (`"1 unread"` both) — parity confirmed on the bridged transport. `snapshots/03` vs `00`; `§3`. Caveat: the numeric value stays small because `unread_output_count` resets on each authority drain (~frame cadence), so a snapshot round-trip catches a post-drain value — a *growing* count is not race-free observable, and the separate compositor `tile_unread_count` pill remains `#[serde(skip)]` / pixel-only. |
| 4 | **Composer input** (draft/submit) flows back over the bridge | **PARTIAL — draft ingress exercised; submit needs OS keyboard** | `inject_composer_paste` injected (`injected=true`), producing draft state, which the bridge classifies as `ResidentBridgeInputKind::DraftState` and `drain_resident_grpc_input` **drops** (only `Submit` reaches pending-input). `portal_projection_get_pending_input` correctly returned 0 items. A submit reaching pending-input requires a real OS Enter keypress on the focused bridged composer — no keyboard-free MCP/gRPC substitute exists in this build. `§4`. |
| 5 | **Clean detach / teardown** | **PASS** | `portal_projection_detach` on the bridged projection → observer snapshot shows `0` tiles and `0` portal_surfaces (bridge tombstone removed the materialised tile). `snapshots/99`; `§5`. Verified on a freshly-restarted runtime (clean scene) to avoid leftover-projection confounds. |

## Product observations (reported, not filed)

- **No product bug found in the hud-hfuxy routing itself** — routing, streaming (units A–D rendered
  over the bridge), the rendered unread indicator, and detach/teardown all behave exactly as specified
  over the bridged transport.
- **Observability notes (not regressions):** the per-projection *transport* is excluded from the wire
  by design (WM-S2b), so routing is proven via the materialised tile's owning `namespace`. The numeric
  compositor `tile_unread_count` pill lives in a `#[serde(skip)]` overlay and is pixel-only, but the
  portal's *rendered* unread indicator is present in the `TextMarkdown` node content and was used for
  the #1107 parity check. The remaining gap is composer **submit** ingress, which needs reliable OS
  keyboard injection — the same limitation prior text-stream rounds recorded (`liveverify-20260710-0955`
  pncm3/2v8br/acfvp).

## Hygiene

Placeholders only (`windows-vm.example`, `proxmox-host.example`, `hud-user`/`admin-user`,
`agent-alpha`, `resident-grpc-portal`). No PSK, no real IPs/hostnames (loopback `127.0.0.1` is
the bridge self-target and is intentional), no owner tokens, no full-desktop captures. The host
was restored to its default bridge-disabled config; all live-verify helper files and scheduled
tasks were removed and the scene left empty.
