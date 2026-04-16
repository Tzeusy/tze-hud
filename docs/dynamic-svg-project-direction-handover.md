# Dynamic SVG Upload: Project-Direction Handover

## Session Snapshot
- Date: 2026-04-08 (Asia/Singapore)
- Working branch: `main` (dirty working tree)
- Goal: move from rigid startup-only widget SVG bundles to **bootstrapped static assets + runtime-uploaded SVG assets**, preserving a **two-stage flow** (upload/register first, publish later), with restart-safe persistence and dedup/checksum support.

## Why This Handover Exists
Context window was running short. This document is for a fresh agent to continue the dynamic SVG effort without re-discovery work.

## Progress Log
- Resume verification completed (current session): revalidated `hud-lviq` bead graph and handover consistency after context compaction.
  - Commands: `bd list --status open --json`, `bd dep cycles --json`.
  - Result: all expected dynamic-svg beads present and dependency cycle check clean (`[]`).
- Pass 1 completed: evidence-only `/legends-and-lore` + `/spec-and-spine` drift inventory.
- Pass 2 completed: `/legends-and-lore` RFC deltas drafted for runtime SVG registration + persistence split.
  - Updated RFCs: `0001-scene-contract`, `0005-session-protocol`, `0006-configuration`, `0011-resource-store`.
  - Key deltas: dual-path widget definition registration, widget asset register/result protocol draft, runtime asset store config section, scoped durable SVG asset exception.
- Pass 3 completed: `/spec-and-spine` delta specs drafted to align with Pass 2 RFC direction.
  - Updated specs: widget-system, session-protocol, resource-store, configuration, component-shape-language.
  - Key deltas: explicit WidgetAssetRegister/Result fields, runtime widget asset store config requirement, v1 persistence split in spec, runtime ingest token-resolution path.
- Pass 4 completed: `/legends-and-lore` + `/spec-and-spine` consistency cleanup pass.
  - Updated RFC: `0005-session-protocol` stale field-allocation language cleaned.
  - Key deltas: removed field-34 reserved-buffer wording, removed 47–49 all-reserved wording, aligned EventBatch note to v1 contract.
- Reconciliation pass completed: `/heart-and-soul` synced against Passes 1-4 deltas.
  - Updated doctrine files: `architecture.md`, `v1.md`.
  - Key deltas: explicit resource-lifecycle persistence split and clarified v1 persistence carve-out language.
- Beads deep-dive pass 1 completed: structural hygiene validation for epic `hud-lviq`.
  - Result: required fields, parent links, and priority/type assignments all valid.
- Beads deep-dive pass 2 completed: dependency/sequencing quality pass for epic `hud-lviq`.
  - Result: added critical-path `blocks` dependencies across protocol/store/registry/MCP/tests/docs/report/reconciliation flow.
- Beads deep-dive pass 3 completed: reconciliation/report compliance validation for epic `hud-lviq`.
  - Result: no dependency cycles, report bead present, reconciliation bead correctly depends on all siblings, epic notes updated with decomposition rationale.

## Beads Created
- Epic: `hud-lviq` — Dynamic SVG runtime upload and durable asset store
- Children:
  - `hud-lviq.1` protocol wire-up (`WidgetAssetRegister`/`Result`)
  - `hud-lviq.2` durable runtime widget asset store backend
  - `hud-lviq.3` runtime registry integration for uploaded SVG assets
  - `hud-lviq.4` MCP `register_widget_asset` surface
  - `hud-lviq.5` dedup/persistence/capability/budget test coverage
  - `hud-lviq.6` topology/ops documentation
  - `hud-lviq.7` human-readable implementation report bead
  - `hud-lviq.8` mandatory spec-to-code reconciliation bead (gen-1)

## What Was Completed In This Session
Doctrine updates were applied in `about/heart-and-soul/` to relax the static-only asset model and explicitly support runtime-uploaded SVGs.

### Files Updated
- `about/heart-and-soul/v1.md`
- `about/heart-and-soul/presence.md`
- `about/heart-and-soul/architecture.md`
- `about/heart-and-soul/security.md`
- `about/heart-and-soul/failure.md`

### Key Doctrine Changes Already Landed (Uncommitted)
1. Runtime SVG/widget assets are now first-class in v1 scope (not startup-only).
2. The model is explicitly two-stage:
   - register/upload asset
   - publish parameters/content referencing registered asset
3. Runtime-uploaded assets are documented as durable across restart.
4. Persistence is documented as OS-specific backend wiring (Linux/macOS/Windows) under one semantic contract:
   - content-addressed blobs
   - atomic writes
   - crash-safe startup reindex/reconcile
5. Dedup/identity semantics are documented:
   - strong hash (BLAKE3) as identity/dedup key
   - optional fast transport checksum (CRC32 example) as integrity hint only
6. Security/capabilities and budgets were extended to include runtime asset registration and storage/upload limits.

## Current Repo State
`git status --short` currently includes doctrine, RFC, and OpenSpec deltas:
- `about/heart-and-soul/architecture.md`
- `about/heart-and-soul/failure.md`
- `about/heart-and-soul/presence.md`
- `about/heart-and-soul/security.md`
- `about/heart-and-soul/v1.md`
- `about/legends-and-lore/rfcs/0001-scene-contract.md`
- `about/legends-and-lore/rfcs/0005-session-protocol.md`
- `about/legends-and-lore/rfcs/0006-configuration.md`
- `about/legends-and-lore/rfcs/0011-resource-store.md`
- `openspec/changes/component-shape-language/specs/widget-system/spec.md`
- `openspec/changes/component-shape-language/specs/configuration/spec.md`
- `openspec/changes/component-shape-language/specs/component-shape-language/spec.md`
- `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
- `openspec/changes/v1-mvp-standards/specs/resource-store/spec.md`
- `docs/dynamic-svg-project-direction-handover.md`

No code changes have been made yet for runtime SVG upload/persistence/dedup.

## Explicit /project-shape Mapping
This section maps status across the four project-shape pillars.

### 1) Doctrine (`about/heart-and-soul/`) — **Reconciled for Dynamic-SVG Scope**
- Status: aligned with RFC/spec direction for this feature.
- Completed: dynamic-SVG doctrine language reconciled across `v1.md`, `presence.md`, `architecture.md`, `security.md`, `failure.md`.
- Residual note: broader non-feature protocol-document drift remains in RFC 0005 examples, but doctrine no longer contradicts the dynamic-SVG contract.

### 2) Design Contracts (`about/legends-and-lore/`) — **Updated For Dynamic-SVG Scope**
- Status: Pass 2 + Pass 4 complete and reconciled with doctrine direction.
- Completed:
  - runtime widget register/upload contract in session-protocol RFC
  - startup + runtime dual-path widget registry language
  - runtime widget asset config section (`[widget_runtime_assets]`)
  - persistence split (scene-node ephemeral vs runtime widget SVG durable)
- Follow-up needed:
  - normalize error vocabulary across RFC 0005 and RFC 0011 during implementation

### 3) Capability Specs (`openspec/`) — **Updated For Dynamic-SVG Scope**
- Status: Pass 3 complete; reconciled with doctrine/contracts for this feature.
- Completed:
  - widget-system spec now defines runtime registration/upload requirement
  - session-protocol spec now documents WidgetAssetRegister/Result field allocations and scenarios
  - resource-store spec now reflects v1 persistence split + IMAGE_SVG inclusion for widget path
  - configuration spec now adds `[widget_runtime_assets]`
  - component-shape-language spec now allows runtime-ingest token substitution path
- Follow-up needed:
  - explicit spec-to-current-code divergence notes as implementation begins

### 4) Topology (`about/lay-and-land/`) — **Not Updated Yet**
- Status: pending.
- Required: map where runtime SVG durable store lives (runtime crate(s), config, storage paths, startup index path, API entrypoints).

## Explicit /project-direction Flow
Use this exactly as next-session execution order.

### Phase 0: Context Framing
- Focus: feature evaluation + decomposition for runtime dynamic SVG assets.
- Non-negotiables:
  - two-stage upload/register then publish
  - restart durability
  - dedup without full payload resend when hash already present
  - capability- and budget-governed behavior

### Phase 1: Spec Scan (Already Run)
Command already executed:
```bash
bash /home/tze/.dotfiles/genai/skills/personal/project-direction/scripts/spec-scan.sh /home/tze/gt/tze_hud/mayor/rig
```
Observed highlights:
- `openspec/` present and active.
- `about/` pillar structure present (`heart-and-soul`, `legends-and-lore`, `lay-and-land`).
- Widget-related work already active in repo history.

### Phase 2: Targeted Investigation (Next Session)
Run parallel evidence gathering before editing specs/code:
1. Locate current widget publish/register/protocol surfaces in code.
2. Locate current asset loading and any existing durable store abstraction.
3. Locate existing checksum/hash utilities used elsewhere (if any).
4. Locate capability enforcement and per-agent budget accounting hooks.
5. Locate startup reconciliation/index rebuild paths (if any) to reuse.

### Phase 3: Work Plan (Spec-First)
Do not code first. Stage this work as:
1. Update doctrine leftovers (small cleanup pass).
2. Update RFC/design contracts (`about/legends-and-lore/`).
3. Update OpenSpec requirements (`openspec/`).
4. Reconcile and get signoff on docs/spec direction.
5. Implement runtime API + storage + dedup + permissions.
6. Add tests (unit + integration + restart durability + budget/capability failures).
7. Reconcile spec↔code and write report.

### Phase 4: Materialize As Beads
Create an epic plus children. Include mandatory reconciliation and report beads (per `/project-direction` guidance).

Suggested bead structure:
- Epic: `dynamic-svg-runtime-upload-and-durable-store`
- Child: `doctrine-cleanup-dynamic-svg-terminology`
- Child: `rfc-update-runtime-svg-upload-register-contract`
- Child: `openspec-update-two-stage-dynamic-svg-flow`
- Child: `implement-upload-register-api-with-hash-short-circuit`
- Child: `implement-os-specific-durable-svg-blob-store`
- Child: `wire-capability-and-budget-enforcement-for-upload`
- Child: `tests-runtime-svg-persistence-dedup-capability-budget`
- Child: `reconciliation-dynamic-svg-spec-code`
- Child: `generate-epic-report-dynamic-svg`

### Phase 5: Deliverables Expected
- Updated doctrine + RFC + openspec chain coherent.
- Implemented runtime API behavior with persistence and dedup.
- Test evidence for correctness and restart survival.
- Reconciliation report for human review.

## Proposed Runtime Behavior Contract (For Spec/RFC Drafting)
Use this as the baseline proposal to either accept or revise.

### Two-Stage Control Plane
1. **Register/upload call**
   - Accepts metadata including strong hash (BLAKE3).
   - Optionally accepts fast checksum (CRC32) for transport guard.
   - If hash exists: return existing asset handle; payload transfer optional/omitted.
   - If hash missing: stream/store payload, validate, index, return handle.
2. **Publish call**
   - Only references pre-registered asset/widget type/instance and lightweight parameters.
   - No large SVG payload on publish path.

### Persistence Contract
- Asset blobs survive process restart.
- Startup scans durable store and rebuilds index if needed.
- Writes are atomic; partial files do not become valid assets.

### Security/Budget Contract
- Upload/register requires explicit capability separate from publish.
- Storage/upload consumption enforces per-agent and global budgets.
- Exceeding budget returns structured, stable error codes.

## Concrete Checklist For Subsequent Sessions

### A. Immediate Start Checklist
- [ ] Re-open this handover and current diffs.
- [ ] Re-run `git status --short` and ensure only intended files are dirty.
- [ ] Do one final `about/heart-and-soul/` grep for stale static-only wording.

### B. /project-shape Checklist
- [x] Doctrine: complete consistency cleanup in heart-and-soul.
- [x] Design contracts: add/update legends-and-lore RFC(s) for runtime upload + persistence.
- [x] Capability specs: update openspec requirements and acceptance criteria.
- [ ] Topology: document where durable store and index paths live.

### C. /project-direction Checklist
- [ ] Run investigation pass with evidence links to files/functions.
- [x] Produce prioritized chunk plan with dependencies.
- [x] Create beads epic + child issues + reconciliation/report beads.
- [x] Sequence work spec-first before implementation.

### D. Implementation Checklist
- [ ] Add runtime register/upload API surface (separate from publish).
- [ ] Add hash short-circuit path (no full payload when already present).
- [ ] Add durable local blob store with OS-specific path wiring.
- [ ] Add startup reindex/reconcile path.
- [ ] Enforce capability and storage/upload budgets on upload/register path.
- [ ] Return stable structured errors for hash mismatch/budget/permission failures.

### E. Testing Checklist
- [ ] Unit tests: hash identity + dedup behavior.
- [ ] Unit tests: checksum mismatch handling.
- [ ] Unit tests: capability denied / budget exceeded.
- [ ] Integration tests: two-stage API flow correctness.
- [ ] Integration tests: restart and rehydrate previously uploaded assets.
- [ ] Integration tests: publish path remains low-bandwidth (no SVG payload required).

### F. Reconciliation & Handoff Checklist
- [ ] Reconcile doctrine ↔ RFC ↔ spec ↔ code.
- [ ] Write a human-readable reconciliation/report doc under `docs/`.
- [ ] Ensure bead statuses reflect true completion.

## Open Questions To Resolve Early
1. What exact protocol shape should upload/register use (new gRPC messages vs extending existing widget APIs)?
2. Should register API support metadata-only preflight (`hash`, `size`) then conditional payload stream?
3. Should hash algorithm be configurable or fixed to BLAKE3 in v1?
4. What retention/GC policy applies to durable SVG blobs (never GC, refcounted GC, TTL GC)?
5. How should ownership/ACLs work when multiple agents reference same hash-identical asset?
6. What is the canonical error-code taxonomy for upload/register failures?

## Risks
- Spec drift risk if code starts before legends-and-lore + openspec updates.
- Hidden doctrine contradiction risk (resource lifecycle wording still partly static-oriented).
- Cross-platform path/permissions bugs for durable storage.
- Budget enforcement ambiguity if store quotas are not defined with exact semantics.

## Useful Commands For Next Agent
```bash
# Current diff scope
git status --short
git diff -- about/heart-and-soul/v1.md about/heart-and-soul/presence.md about/heart-and-soul/architecture.md about/heart-and-soul/security.md about/heart-and-soul/failure.md

# Re-run project-direction scan
bash /home/tze/.dotfiles/genai/skills/personal/project-direction/scripts/spec-scan.sh /home/tze/gt/tze_hud/mayor/rig

# Scan for stale startup-only language
rg -n "static in v1|startup-only|bundle load time|cannot create widget|no runtime widget" about/heart-and-soul about/legends-and-lore openspec
```

## Definition Of Done For This Epic (High-Level)
- The project doctrine/spec/contracts all state the same dynamic SVG model.
- Runtime accepts upload/register with hash-aware dedup and separate publish path.
- Runtime-uploaded SVG assets survive restart on supported OS targets.
- Capability + budget enforcement is tested and observable.
- A new agent can continue work with this handover without reconstructing context.
