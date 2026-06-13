# docs/ — tze_hud Documentation Index

This directory holds operational and episodic documentation for the tze_hud project. It is NOT the place to find normative reference material; for that, see the five pillars in `about/` and the capability specs in `openspec/`.

## Quick Navigation

| Need | Go to |
|------|-------|
| Project doctrine, vision, non-negotiables | `about/heart-and-soul/` |
| RFCs, wire contracts, state machines | `about/legends-and-lore/rfcs/` |
| Capability requirements and scenarios | `openspec/specs/` |
| Topology, component maps, data flow | `about/lay-and-land/` |
| Engineering quality bar | `about/craft-and-care/engineering-bar.md` |
| This directory (operational + episodic artifacts) | `docs/` (here) |

---

## Normative Reference (`docs/`)

These subdirectories contain reference material that agents and humans may need to consult during implementation or operations. Content here is authoritative for its specific domain.

### `audits/`

Library and dependency audits. Decisions recorded here have binding effect on implementation choices.

| File | Subject |
|------|---------|
| `20260612_project_review.md` | Full project audit — §4 topology, §5.13 doc estate, §8 risks, §9 roadmap, §10 shape |
| `20260612_enforcement_machinery_epic_report.md` | Epic closeout report for enforcement-machinery remediation (hud-1aswu) |
| `20260612_reconciliation_gen1.md` | Gen-1 reconciliation for enforcement-machinery audit remediation (hud-1aswu.6) |
| `20260613_validation_framework_verification.md` | Validation-framework conformance audit and OpenSpec sync verification (hud-olxxd) |
| `android-gstreamer-sdk-build-spike.md` | Android GStreamer SDK build investigation |
| `cpal-audio-io-crate-audit.md` | `cpal` audio I/O crate fitness |
| `gstreamer-media-pipeline-audit.md` | GStreamer media pipeline audit |
| `gstreamer-windows-ci-bootstrap.md` | GStreamer CI bootstrap on Windows |
| `ios-videotoolbox-alternative-audit.md` | iOS VideoToolbox alternative investigation |
| `statig-state-machine-audit.md` | E26 `statig` library audit (acceptable for internal state machines only) |
| `webrtc-rs-audit.md` | `webrtc-rs` crate audit |
| `webrtc-sfu-fallback-audit.md` | WebRTC SFU fallback strategy |

### `decisions/`

Architectural decision records. Each file captures a binding design choice and its rationale.

| File | Subject |
|------|---------|
| `codec-cve-sandbox-hardening-v3.md` | Codec CVE sandbox hardening decision |
| `e24-in-process-worker-posture.md` | E24 in-process worker posture decision |
| `relay-resource-url-agent-exposure.md` | Relay resource URL agent exposure scope |

### `operations/`

Runbooks for maintaining the development and production infrastructure.

| File | Subject |
|------|---------|
| `beads-coordination-backup.md` | Beads Dolt backup setup (requires operator-owned destination) |
| `tzehouse-windows-recovery.md` | Windows HUD host recovery runbook |

### `ci/`

CI infrastructure setup guides.

| File | Subject |
|------|---------|
| `android-gstreamer-bootstrap.md` | Android GStreamer CI bootstrap |
| `safari-simulcast-interop-runner.md` | Safari simulcast interop CI runner |
| `windows-d18-runner-setup.md` | Windows D18 CI runner setup |

### `design/`

Design documents and migration plans that inform implementation work.

| File | Subject |
|------|---------|
| `turn-client-integration.md` | TURN client integration design |
| `tzehouse-windows-gpu-scheduling.md` | Windows GPU scheduling design |
| `webrtc-rs-0.17-to-0.20-migration-plan.md` | `webrtc-rs` migration plan |

### `testing/`

Testing guides and harness documentation.

| File | Subject |
|------|---------|
| `simulcast-interop-plan.md` | Safari simulcast interop testing plan |

---

## Episodic Artifacts (`docs/`)

These subdirectories contain artifacts produced during specific work episodes. They record what happened, not what must be done. Use them for historical context, not as normative guidance.

> **Archive policy**: Episodic artifacts are never deleted. They are append-only records. Large binary or JSON evidence files belong in `docs/evidence/` and should be cross-referenced from the relevant report or reconciliation file.

### `reports/`

Investigation reports, direction documents, validation records, and coverage reports produced during feature and bug work. Files are named by bead ID or date to aid traceability.

~45 files. Examples:

| File | Subject |
|------|---------|
| `20260612_project_direction.md` | Project direction from 2026-06-12 audit |
| `windows_perf_baseline_2026-05.md` | First Windows reference-hardware perf baseline |
| `cooperative_hud_projection_gen2_reconciliation_20260510.md` | Gen-2 reconciliation: runtime-native readback substitution |
| `validation_operations_extraction_decision_20260425.md` | Decision to extract v1 validation-operations backlog into standalone OpenSpec |

### `reconciliations/`

Spec-to-code and spec-to-spec reconciliation documents. Produced after major feature epics; record gap analysis, coverage maps, and follow-up beads.

~47 files. The canonical seam artifact for policy wiring is `policy_wiring_seam_contract.md`.

### `evidence/`

Raw test artifacts, screenshots, JSON payloads, and benchmark outputs tied to specific validation runs. Organized by bead or feature name.

Total size: ~11 MB. Subdirectories: `cooperative-hud-projection/`, `external-agent-projection-authority/`, `hud-9m47l/`, `text-stream-portals/`. Contains committed evidence for specific bead validation runs.

> Note: `test_results/` at the repo root is gitignored. Artifacts that must be committed are moved to `docs/evidence/` and force-added with `git add -f`.

---

## Other Standalone Docs

A few standalone documents live directly under `docs/` rather than a subdirectory. These are typically feature-scoped design notes or one-off coverage analyses that predate the current directory structure.

| File | Subject |
|------|---------|
| `text-stream-refinement.md` | Text-stream portal design notes (the "no bottom-chat input" decision lives here) |
| `component-profile-authoring.md` | Component profile authoring guide |
| `component-shape-language-coverage.md` | Shape language coverage analysis |
| `dynamic-svg-implementation-report.md` | Dynamic SVG implementation report |
| `dynamic-svg-project-direction-handover.md` | Dynamic SVG direction handover doc |
| `exemplar-dashboard-tile-coverage.md` | Dashboard tile exemplar coverage |
| `exemplar-dashboard-tile-user-test.md` | Dashboard tile user-test record |
| `exemplar-manual-review-checklist.md` | Shared manual review checklist for all exemplars |
| `exemplar-notification-coverage.md` | Notification exemplar coverage |
| `exemplar-presence-card-coverage.md` | Presence Card exemplar coverage |
| `exemplar-presence-card-user-test.md` | Presence Card user-test record |
| `exemplar-progress-bar-coverage.md` | Progress bar exemplar coverage |
| `hud-ltgk-reconciliation.md` | hud-ltgk bead reconciliation |
| `20260511_goals.md` | Goals doc from 2026-05-11 |

---

## Five-Pillar Doc Estate (outside `docs/`)

The primary normative documentation lives OUTSIDE this directory:

```
about/
  heart-and-soul/      ← Doctrine: vision, principles, v1 scope, validation arch
  legends-and-lore/    ← Design contracts: 14 RFCs, reviews, reconciliations
  lay-and-land/        ← Topology: component maps, data flow, deployment
  craft-and-care/      ← Engineering standards: quality bar, perf budgets
openspec/
  specs/               ← Normative capability specs (WHEN/THEN scenarios)
  changes/             ← Active OpenSpec change directories
  changes/archive/     ← Archived (shipped) changes
```

Use the local skills (`/heart-and-soul`, `/legends-and-lore`, `/spec-and-spine`, `/lay-and-land`, `/craft-and-care`) for selective, context-appropriate loading of these pillars.
