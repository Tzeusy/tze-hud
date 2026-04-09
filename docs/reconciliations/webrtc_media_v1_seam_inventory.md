# WebRTC/Media V1 Seam Inventory (WM-S0)

Date: 2026-04-09
Issue: `hud-nn9d.5`
Parent epic: `hud-nn9d`

## Purpose

Enumerate all repository seams that must be explicit before authoring the first bounded post-v1 media ingress capability spec.

This inventory is spec-planning only. It does not authorize runtime media implementation in v1.

## Bounded Slice Framing

Target slice remains: one-way post-v1 media ingress into runtime-owned zone surfaces, with v1 defaults unchanged (`media off`, no live WebRTC/GStreamer runtime path).

Non-goals for this seam pass:
- Bidirectional AV session semantics
- Audio routing/mixing policy
- Multi-feed composition and adaptive bitrate orchestration

## Seam Inventory

| Seam ID | Required Surface | Repo Seams (Evidence) | Hidden Assumption Surfaced | Downstream Disposition |
|---|---|---|---|---|
| WM-S0-1 | Protocol and signaling shape | `session.proto` has `ZonePublish` on the main session stream (`crates/tze_hud_protocol/proto/session.proto:525`) while scene mutations also have `PublishToZoneMutation` (`crates/tze_hud_protocol/proto/types.proto:328`). Post-v1 embodied/media signaling is reserved (`openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md:698`). | The first media slice is currently ambiguous between extending `ZonePublish`, extending mutation schema, or introducing separate media RPC/signaling. | `WM-S2a` (signaling-shape decision) after `WM-S1`.
| WM-S0-2 | Proto/schema and snapshot semantics | Scene model carries `expires_at_wall_us`, `content_classification`, and breakpoints in zone records (`crates/tze_hud_scene/src/graph.rs:2777`). gRPC `ZonePublish` path hardcodes `expires_at_wall_us: None` and `content_classification: None` (`crates/tze_hud_protocol/src/session_server.rs:3722`). `ZoneContent` proto omits media payload variants like `VideoSurfaceRef` (`crates/tze_hud_protocol/proto/types.proto:198`). Snapshot currently emits empty `zone_instances` (`crates/tze_hud_scene/src/graph.rs:3641`). | Wire/snapshot contracts are not yet equivalent to in-memory scene semantics; media ingress would otherwise inherit silent metadata loss and unclear reconnect behavior. | `WM-S2b` (schema/snapshot delta spec) after `WM-S2a`.
| WM-S0-3 | Zone transport and layer-attachment semantics | Zone schema includes `transport_constraint` + `WebRtcRequired` (`crates/tze_hud_scene/src/types.rs:1509`) and `layer_attachment` (`openspec/changes/v1-mvp-standards/specs/scene-graph/spec.md:242`), but proto zone definition only carries `ephemeral` (no transport/layer fields) (`crates/tze_hud_protocol/proto/types.proto:295`). Scene publish path is global by zone name, not tab-scoped instance resolution (`crates/tze_hud_scene/src/graph.rs:2712`) while spec language is instance-oriented (`openspec/changes/v1-mvp-standards/specs/scene-graph/spec.md:199`). | Transport and layer policy are partially modeled but not consistently propagated/enforced across scene/protocol surfaces; tab instance semantics are implicit. | `WM-S2c` (zone contract) and `WM-S3c` (compositor contract) after `WM-S2b`.
| WM-S0-4 | Config/profile surfaces | Config raw schema includes `max_media_streams` (`crates/tze_hud_config/src/raw.rs:71`), but profile ceiling logic explicitly notes no `DisplayProfile` field exists yet (`crates/tze_hud_config/src/profile.rs:321`). Lease budgets enforce `max_concurrent_streams = 0` in v1 (`openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md:157`, `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md:167`). | Admission ceilings for media streams are declared at config layer but not fully represented in resolved profile/runtime budget surfaces. | `WM-S3` (activation gate + budgets) and `WM-S3b` (policy controls).
| WM-S0-5 | Compositor/render contract | Scene allows `ZoneContent::VideoSurfaceRef` (`crates/tze_hud_scene/src/graph.rs:3577`) and tests accept schema-only publish (`crates/tze_hud_scene/tests/zone_ontology.rs:709`), while protocol conversion omits encoding for `StaticImage`/`VideoSurfaceRef` payloads (`crates/tze_hud_protocol/src/convert.rs:330`). Renderer has zone render branches for text/notification/status/static-color/image but no `VideoSurfaceRef` render path (`crates/tze_hud_compositor/src/renderer.rs` search results). | "Schema accepted" currently does not imply renderable media behavior; present-time, texture ownership, and degradation fallback are unspecified. | `WM-S3c` (compositor contract) and then post-gate implementation tranche.
| WM-S0-6 | Validation strategy | Validation spec requires synthetic media inputs and media SSIM thresholds (`openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:36`, `openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:55`) and includes `sync_group_media` in scene corpus (`openspec/changes/v1-mvp-standards/specs/validation-framework/spec.md:161`), but there is no bounded media-ingress-specific acceptance matrix yet. | Existing validation architecture is capable, but no first-slice ingress rehearsal contract exists (protocol + privacy + operator + degradation combined). | `WM-S4` (media rehearsal scenarios/pass-fail thresholds).
| WM-S0-7 | Privacy/operator controls | Privacy/redaction + quiet-hours are normative and hot-reloadable (`openspec/changes/v1-mvp-standards/specs/configuration/spec.md:216`, `openspec/changes/v1-mvp-standards/specs/configuration/spec.md:229`, `openspec/changes/v1-mvp-standards/specs/configuration/spec.md:264`), and policy stack defines zone ceiling behavior (`openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md:109`). However gRPC zone publish currently drops per-publication classification metadata (`crates/tze_hud_protocol/src/session_server.rs:3723`). | Media ingress would otherwise bypass a declared per-publication privacy control path on one transport. | `WM-S3b` (privacy/operator/enablement policy) plus schema alignment in `WM-S2b`.
| WM-S0-8 | Default-off activation gates | v1 doctrine defers GStreamer/WebRTC (`about/heart-and-soul/v1.md:115`), runtime kernel defers media worker pool spawn (`openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md:374`), and lease budget keeps streams at zero (`openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md:167`). | Activation conditions are currently distributed across doctrine/spec/implementation with no single post-v1 gate contract tying config, capability, transport, and observability prereqs together. | `WM-S3` (single activation-gate contract) and explicit defer markers (`WM-D*`). |

## Downstream Spec Conversion Map

| Inventory seam | Next spec bead(s) | Status in tracker |
|---|---|---|
| Bounded capability contract envelope | `hud-nn9d.6` / `WM-S1` | `hud-nn9d.6` exists (closed) |
| Signaling-shape decision | `hud-nn9d.7` / `WM-S2a` | `hud-nn9d.7` exists (closed) |
| Proto/schema + snapshot deltas | `hud-nn9d.8` / `WM-S2b` | `hud-nn9d.8` (this issue) |
| Zone transport/layer/reconnect contract | `WM-S2c` | Not yet instantiated |
| Runtime activation gate + budgets | `WM-S3` | Not yet instantiated |
| Privacy/operator/enablement policy | `WM-S3b` | Not yet instantiated |
| Compositor render/degradation contract | `WM-S3c` | Not yet instantiated |
| Validation rehearsal scenarios | `WM-S4` | Not yet instantiated |
| Doctrine/readme alignment | `WM-S5`, `WM-S6` | Not yet instantiated |

Coordinator note: these bead names and ordering come from `docs/reconciliations/webrtc_media_v1_backlog_materialization.md`.

## Explicit Deferrals (Remain Deferred)

These are explicitly outside the first bounded ingress slice and should stay deferred (`WM-D*`):
- Bidirectional AV/WebRTC call/session negotiation and embodied presence
- Audio routing and mixing policy engine
- Multi-feed compositing and adaptive bitrate orchestration

## Planning Guardrails Before Writing `hud-nn9d.6`

1. Treat protocol shape (`WM-S2a`) as a first-class decision, not an implementation detail.
2. Do not assume current proto snapshots preserve scene privacy/expiry metadata.
3. Do not assume zone transport/layer fields are wire-visible just because scene types carry them.
4. Keep v1 defaults unchanged (`max_concurrent_streams = 0`, media pool not spawned) until `WM-S3` criteria are met.
