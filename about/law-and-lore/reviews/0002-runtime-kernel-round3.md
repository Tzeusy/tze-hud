# Review: RFC 0002 Runtime Kernel — Round 3 of 4
## Cross-RFC Coherence and Integration

**Issue:** rig-5vq.17
**Date:** 2026-03-22
**RFC reviewed:** about/law-and-lore/rfcs/0002-runtime-kernel.md
**Doctrine read:** architecture.md, failure.md, security.md
**RFCs cross-checked:** RFC 0005 (Session Protocol), RFC 0008 (Lease Governance), RFC 0009 (Policy & Arbitration)

---

## Scores

| Dimension | Score | Rationale |
|---|---|---|
| Doctrinal Alignment | 4 | Doctrine holds. All v1-scope notes, sovereignty model, and timing semantics are consistent with architecture.md, failure.md, and security.md. No regression from round 2 fixes. |
| Technical Robustness | 4 | Round 2 MUST-FIX items (T-1 through T-5) remain in place and are correct. Three new cross-RFC items found (T-7, T-8, T-9); all applied directly to the RFC in this round. |
| Cross-RFC Consistency | 4 | Two MUST-FIX cross-RFC inconsistencies resolved. One terminology clarification (T-9) also applied. RFC 0002 now fully aligned with RFC 0008 and RFC 0009 as they exist after those RFCs landed. |

All dimensions are at or above 3. All MUST-FIX items have been applied directly to the RFC.

---

## Doctrine Files Reviewed

### architecture.md
- Screen sovereignty and three-protocol-plane model unchanged and still correctly expressed in RFC 0002.
- GPU failure path fix (T-7) is consistent with the "runtime owns pixels" doctrine: the runtime must inform the viewer of its termination rather than disappearing silently.
- Sort direction clarification (T-8) is an implementation detail, not a protocol-plane concern.

### failure.md
- Degradation ladder (five levels) maps correctly to the six doctrine axes, with deferred axes (media streams, audio) properly noted.
- Level 5 "visually suppressed" wording (T-9) better aligns with failure.md's distinction between rendering degradation and lease governance failure.

### security.md
- Resource governance model is consistent with RFC 0008 §4. No new gaps.
- Budget enforcement three-tier ladder in §5.2 matches RFC 0008 §4.3 (the authoritative reproduction).

---

## Findings Applied in This Round

### MUST-FIX (all addressed)

**T-7: RFC 0002 §1.4 and §7.3 GPU failure path contradicts RFC 0009 §5 resolution**

- **Location:** §1.4 (fatal GPU error emergency path, line "Fatal GPU errors..."), §7.3 (step 4)
- **Problem:** RFC 0002 §1.4 stated "exit immediately without waiting for agents" as the response to fatal GPU errors. RFC 0002 §7.3 step 4 stated "trigger graceful shutdown with non-zero exit code" for device truly lost. RFC 0009 §5 landed after round 2 and explicitly resolved the RFC 0002/RFC 0007 conflict: the correct behavior is a two-phase procedure — (1) attempt safe mode entry with `CRITICAL_ERROR` reason to inform the viewer before exit, (2) then trigger graceful shutdown. RFC 0002 was not updated when RFC 0009 landed. An implementation following RFC 0002 §1.4 verbatim would produce a silent process exit on fatal GPU error, violating both the RFC 0009 resolution and the doctrine principle that the human must always be informed of critical failures.
- **Fix applied:**
  - §1.4 emergency path sentence: replaced "exit immediately without waiting for agents" with the two-phase procedure (safe mode entry → graceful shutdown), with a cross-reference to §7.3 and RFC 0009 §5.
  - §7.3 step 4: replaced "trigger graceful shutdown (§1.4) with non-zero exit code" with the full two-phase procedure matching RFC 0009 §5.3's required change verbatim.
- **Doctrine rationale:** RFC 0009 §5.2 explicitly states "RFC 0009 is authoritative." RFC 0002 deferred to RFC 0009's resolution. The fix brings RFC 0002 into compliance.

---

**T-8: `lease_priority` sort direction expressed as `DESC` in §5.2 — contradicts RFC 0008 §2.2 canonical formulation**

- **Location:** §5.2 frame-time guardian, shed tile sort step
- **Problem:** RFC 0002 §5.2 describes the sort as `(lease_priority DESC, z_order DESC)` with an inline note explaining that "DESC means lower numeric values appear first." RFC 0008 §2.2 landed after round 2 and established the canonical formulation: `(lease_priority ASC, z_order DESC)` with the explanation that numerically lower = higher priority = preserved first. RFC 0008 §2.2 explicitly notes the prior phrasing ambiguity: "RFC 0002's `DESC` for `lease_priority` means 'sorted descending by value, so lower numeric values appear first' — equivalent to `ASC` when the goal is 'lower value = higher importance.'" An implementation reading RFC 0002 §5.2 with a conventional `DESC` interpretation (largest values first, not smallest) would shed the wrong tiles — it would shed the highest-priority tiles first.
- **Fix applied:** §5.2 shed step rewritten to use `(lease_priority ASC, z_order DESC)` with a plain English description matching RFC 0008 §2.2. Added cross-reference to RFC 0008 §2.2 as the authoritative formulation. Degradation Ladder Level 4 bullet in §6.2 also updated to use `ASC` formulation with cross-reference.
- **Doctrine rationale:** A sort inversion at Level 4/5 would cause the highest-priority agent tiles (the ones the runtime is supposed to protect) to be shed first under load. This is a correctness-critical issue.

---

### SHOULD-FIX (addressed)

**T-9: Level 5 "tiles suspended" terminology collides with `SUSPENDED` lease state defined in RFC 0008 §3**

- **Location:** §6.2 Degradation Ladder, Level 5 bullet
- **Problem:** RFC 0002 §6.2 Level 5 reads "All other agent tiles suspended (not revoked — leases remain valid)." RFC 0008 §3.1 defines `SUSPENDED` as a formal lease state with a specific state machine transition (triggered by safe mode entry, not by degradation). RFC 0008 §3.3 explicitly clarifies that Level 5 does NOT change lease state to `SUSPENDED` — it is "a rendering suspension only." The word "suspended" in RFC 0002 §6.2 will cause implementors to incorrectly apply the `SUSPENDED` lease state during Level 5 degradation, which would (among other problems) cause agents to receive `SAFE_MODE_ACTIVE` errors and session notifications that have nothing to do with safe mode.
- **Fix applied:** Level 5 bullet text changed from "All other agent tiles suspended (not revoked — leases remain valid)" to "All other agent tiles visually suppressed (rendering-only suppression — leases remain ACTIVE, NOT in SUSPENDED state; see RFC 0008 §3.3)."
- **Doctrine rationale:** The lease state machine in RFC 0008 is the authoritative governance model. Overloading "suspended" across rendering and governance contexts creates a fatal implementation ambiguity.

---

## Cross-RFC Consistency — Full Check (Round 3)

This section records the cross-RFC verification against all RFCs that landed since round 2 (RFC 0008, RFC 0009) and the RFC 0005 batch coalescing fix (PR #46, rig-2kz).

### RFC 0008 (Lease Governance) — landed since round 2

| RFC 0002 section | RFC 0008 section | Status |
|---|---|---|
| §5.2 budget enforcement three-tier ladder | §4.3 (authoritative reproduction) | Consistent — RFC 0008 reproduces RFC 0002 §5.2 exactly and adds detail |
| §5.1 `AgentResourceState` / `BudgetState` | §4.4 resource cleanup on revocation | Consistent — both describe the same frame-tick revocation procedure; RFC 0008 adds `max_active_leases` check detail |
| §5.2 shed sort `(lease_priority DESC, z_order DESC)` | §2.2 canonical `(lease_priority ASC, z_order DESC)` | **Fixed (T-8)** |
| §6.2 Level 5 "tiles suspended" | §3.3 Level 5 is NOT lease SUSPENDED | **Fixed (T-9)** |
| §4.3 `max_active_leases` default=8 | §4.1 `ResourceBudget.max_active_leases` default matches §4.3 | Consistent |
| §4.3 `max_tiles` default=8 | §4.1 `ResourceBudget.max_tiles` default matches | Consistent |
| §4.3 `max_texture_bytes` default=256MiB | §4.1 `texture_bytes_total` default matches | Consistent |

### RFC 0009 (Policy & Arbitration) — landed since round 2

| RFC 0002 section | RFC 0009 section | Status |
|---|---|---|
| §7.3 GPU device lost step 4 (shutdown) | §5.3 required change to §7.3 | **Fixed (T-7)** |
| §1.4 fatal GPU error emergency path | §5.2 resolution: safe mode before shutdown | **Fixed (T-7)** |
| §6.2 degradation ladder | §6.1 degradation as arbitration gate (Step 7) | Consistent — RFC 0009 §6 references RFC 0002 §6; no contradiction |
| §5.2 budget enforcement / revocation | §2.7 transactional mutations never shed | Consistent — RFC 0002 §5.2 does not shed transactional mutations (budget enforcement is rejection, not shedding) |
| §6.4 DegradationEvent | §6.3 agent backpressure on DegradationEvent | Consistent — RFC 0009 §6.3 calls out RFC 0002 §6.4 correctly |

### RFC 0005 batch coalescing fix (PR #46, rig-2kz) — landed since round 2

| RFC 0002 section | RFC 0005 / PR #46 | Status |
|---|---|---|
| §3.2 Stage 3 "Batches are never coalesced" | PR #46 added this clause to RFC 0002 directly | Consistent — the clause is present and correct; states coalescing applies only to outbound SceneEvent fan-out, not inbound MutationBatch |
| §6.2 Level 1 Coalesce bullet | Updated in PR #46 to clarify inbound/outbound distinction | Consistent — bullet now explicitly states inbound MutationBatch messages are never coalesced |
| §6.2 Level 1 / RFC 0005 §3.2 (outbound SceneEvent coalescing) | Coalescing is outbound only | Consistent |

### RFC 0001 (Scene Contract)
- No new gaps. `SceneMutation`, `SceneId`, `MutationBatch` usage consistent with round 2.

### RFC 0003 (Timing Model)
- T-6 from round 2 (Appendix A `stage_durations_us` vs `FrameTimingRecord`) note still in place and correct.

### RFC 0005 (Session Protocol)
- Shutdown sequencing (T-3 from round 2) still correct. RFC 0005 grace period (30s session-level) and RFC 0002 drain timeout (500ms process-level) operate on independent state machines.
- `SceneId` usage in `MutationBatch.batch_id` / `MutationBatch.lease_id` consistent with RFC 0005 round 7 (rig-de2).

### RFC 0006 (Configuration)
- No new gaps. `redaction_style` ownership conflict was resolved in RFC 0009 §3.2 (belongs in `[privacy]`, not `[chrome]`). RFC 0002 does not reference `redaction_style` — no impact.

### RFC 0007 (System Shell)
- RFC 0007 §5.1 describes automatic safe mode entry on GPU device loss. RFC 0002's updated §1.4 and §7.3 now cross-reference RFC 0007 §5.1 correctly, consistent with RFC 0009 §5.3.

---

## Open Questions (Carried Forward)

The following items from round 2 remain open and are not addressed in this round:

- **C-3:** `EventNotification` ring buffer can drop critical revocation events under overflow. Still acceptable for v1; post-v1 consideration.
- **C-4:** Frame-time guardian threshold arithmetic is tight at the boundary under OS scheduling noise. Suggested addition to §10 Open Questions (not applied in round 2 or 3).
- **C-5:** `HeadlessSurface` pixel format `Rgba8UnormSrgb` may differ from windowed path `Bgra8UnormSrgb`; Layer 1 test pixel readback helpers should normalize. Deferred.

---

## Discovered Issues (Out of Scope — for coordinator)

None discovered in this pass beyond what is fixed above.

---

## Summary

RFC 0002 is structurally sound after three rounds of review. Round 3 focused on cross-RFC coherence against RFCs 0008, 0009, and the PR #46 batch coalescing fix. Two MUST-FIX issues were found and resolved:

1. **T-7:** GPU failure path (§1.4 and §7.3) was out of sync with RFC 0009 §5's authoritative resolution. Both locations now follow the two-phase safe mode → graceful shutdown procedure.
2. **T-8:** `lease_priority` sort direction expressed as `DESC` in §5.2 was ambiguous and diverged from RFC 0008 §2.2's cleaner `ASC` formulation. All sort references updated.

One SHOULD-FIX issue was also resolved:

3. **T-9:** Level 5 "tiles suspended" terminology collided with the formal `SUSPENDED` lease state in RFC 0008 §3. Language changed to "visually suppressed" with an explicit cross-reference to RFC 0008 §3.3.

The RFC is ready for round 4 review (Implementation Readiness and API Ergonomics).
