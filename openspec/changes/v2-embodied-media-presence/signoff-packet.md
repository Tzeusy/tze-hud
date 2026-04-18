# v2-embodied-media-presence — Signoff Packet

Status: **pending final signoff** — unblocks task 6.3 (generate v2 beads graph).
Captured: 2026-04-18.
Respondent: Tzeusy (solo operator + parallel LLM agent fleet).

This packet records the decisions taken across the A–G planning Q&A that resolves the abstract proposal/design into a concrete buildable v2 program.

---

## Section A — Scope

| ID | Decision | Notes |
|---|---|---|
| A1 | **All four phases in v2.** Phase 4 gets its own dedicated beads epic to signal separability. | Sequencing: 1→2 strictly sequential; 3 overlaps last month of 2; 4 fans out into parallel sub-epics. |
| A2 | **See reconciliation below.** Initial answer was "bidirectional AV in v3", but A5 kept voice synthesis in scope, implying full bidirectional AV in v2. **Assumed: bidirectional AV in v2 phase 4.** *Pending explicit confirmation.* |
| A3 | Mobile **first-class**; glasses **contract-ready but not exercised in v2**. Upstream-composition and capability-negotiation interfaces must be **generalized** so XReal, Meta Ray-Ban, and AVP can plug in post-v2 without breaking the protocol. | Wire contract: glasses device receives pre-composited frames over WebRTC + returns input events upstream. Glasses never see the scene graph. |
| A4 | **One embodied agent at a time.** Additional embodied requests are queued or rejected; multi-agent embodied presence is post-v2. | |
| A5 | **Non-goals list reduced to a single item.** All expansions listed below are **in scope for v2 phase 4** (one sub-epic each). | See expansions table. |

### A5 — Final non-goals list (v2 will NOT do)

1. **No background audio.** Media plays only when its owning surface is visible/foregrounded.

### A5 — Scope expansions (pulled OUT of non-goals, into phase 4 sub-epics)

| Expansion | Phase-4 sub-epic | Governance addition required |
|---|---|---|
| Recording of media streams | 4a | Consent, retention, indicator, access control, legal review |
| Cloud-relayed media (via SaaS SFU) | 4b | Trust boundary spec, vendor selection |
| Agent-to-agent media (runtime-relayed) | 4e | Per-hop policy enforcement; routing rules |
| Voice synthesis (agent-emitted audio) | 4f | Bundled with bidirectional AV; requires operator-audible indicator |

**Demoted to v3 (post-v2) after project-direction audit:**
- ~~Federation across households/operators (4c)~~ — scope inflation vs. the "one household's screens" thesis; adds cross-operator policy merge + additional GDPR exposure + legal re-review. V2 reserves federation-aware *fields* in data structures but ships no 4c sub-epic.
- ~~External transcoding services (4d)~~ — vendor choice was already deferred "until use case emerges"; that is the cut signal. V2 leaves transcoding abstraction points in `media-plane` spec but ships no 4d sub-epic.

### Phase 4 internal DAG

The remaining phase-4 sub-epics have internal dependencies, not six parallel lanes:

```
4f (bidirectional AV + voice)    ─┐
                                   ├──► 4a (recording, needs audio-emit capture path)
4b (cloud-relay)                  ─┤
                                   └──► 4e (agent-to-agent, needs bidi AV wire + relay)
```

Sub-epic ordering: **{4f, 4b} first (parallel) → {4a, 4e} (parallel)**. No phase-4 sub-epic opens until prior-phase closeout report lands (F33).

### A2/A1 reconciliation (needs confirmation)

Because voice synthesis is in scope (A5 line 6), bidirectional AV is effectively in v2. **Treating A2 as (a): bidirectional AV in v2 phase 4.** Flag if this is wrong — the original answer was (c) v3.

---

## Section B — Embodied semantics

| ID | Decision | Notes |
|---|---|---|
| B6 | *Not explicitly answered.* **Assumed**: embodied is distinguished from resident by (i) durable session identity, (ii) mandatory operator visibility, (iv) strict revocation ≤500 ms with audit, (v) device-aware routing. **Not** by cert-based auth or by forced media capability. *Pending explicit confirmation.* |
| B7 | **Extend existing gRPC bidi stream** with new embodied message types. No parallel transport; no pure-WebRTC control plane. | |
| B8 | **Operator-configurable reclaim window, default 120 s.** Per-session override via policy. | Glasses/mobile sessions can extend; household-visible sessions stay short. |
| B9 | **Device-reboot-persistent identity, with user-initiated cryptographic export/import** for device migration. No cloud identity anchor. | Ritual: operator triggers "pair new device" → export package → import on new device → old device revoked. |
| B10 | **SessionInit flag `presence_level = EMBODIED` + `embodied` lease capability**, both required. | SessionInit flag sets transport behavior (heartbeat, reclaim window); capability gates operator policy. |
| B11 | **On media drop while session survives**: media surface shows last frame with disconnection badge, session continues, control path stays alive. | Matches v1 orphan-badge precedent. |

---

## Section C — Governance & policy

| ID | Decision | Notes |
|---|---|---|
| C12 | **Role-based operators** (owner / admin / member / guest). Federation-aware roles modeled in the data model but not fully enforced in v2. | |
| C13 | **Per-capability grants** with opinionated bundled presets ("household demo", "cloud-collab"). **Runtime config for fundamental on/off (restart), per-session operator dialog for first-use** within enabled capabilities. Dialog remembers per-agent-per-capability for 7 days. | Capabilities: media-ingress, microphone-ingress, audio-emit, recording, cloud-relay, external-transcode, federated-send, agent-to-agent-media. **The dialog + 7-day-remember behavior is RFC-level** (RFC 0008 lease-governance amendment); not a spec-only decision. |
| C14 | **Recording policy**: operator-grants-once consent (i), local disk only (i), 30 d default TTL (operator-configurable), role-based access (ties to C12), **no redaction** (post-v2), **mandatory continuous visible indicator regardless of jurisdiction**. | Legal-jurisdiction enforcement (2-party consent states, EU GDPR) requires pre-ship counsel review (see procurement.md). **Doctrine-destined**: the indicator mandate is a prime ethical directive and lands in `recording-ethics.md` before appearing in any spec. |
| C15 | **Cloud-relay trust boundary**: media payload only (a). Presence-metadata export is a separate capability (`federated_presence_visibility`) requiring explicit operator grant. **Vendor: SaaS for v2 (LiveKit Cloud or Cloudflare Calls)**; self-host is post-v2. | Final vendor pick made at phase 4b kickoff. |
| C16 | **Tiered revocation**: soft first (graceful 500 ms), hard fallback (SIGKILL + audit within 100 ms). Audit records path taken. **Truncated recording kept with `operator_revoked` event in metadata** — not purged. | |
| C17 | **Mandatory audit events**: session lifecycle, capability grants/denials, media admission/teardown/degradation, recording start/stop/access, cloud-relay activation, operator overrides, federation handshakes, agent-to-agent media flows, policy evaluation failures. **Retention 90 d default, operator-configurable. Local append-only log with daily rotation. Schema versioned.** | Log is local-only by default; operator-triggered export is a separate capability. |

---

## Section D — Validation strategy

| ID | Decision | Notes |
|---|---|---|
| D18 | **Dedicated self-hosted GPU runner (b)** for nightly real-decode; PR gates pass with synthetic-only. Codecs: **H.264 + VP9** for v2, AV1 deferred. Reference streams: **fixed library checked into LFS**. Thresholds: glass-to-glass p50 ≤150 ms / p99 ≤400 ms; decode-drop ≤0.5%; lip-sync drift ≤±40 ms; TTFF ≤500 ms. | |
| D19 | **Real-primary device coverage** (1× iPhone, 1× Android, 1× Mac, 1× Windows, 1× Linux) + **cloud device farm for long-tail breadth**. Glasses: own one of each targeted SKU as they become available. Simulators supplementary only. | |
| D20 | **CI cadence matrix**: unit/synthetic per-PR; real-decode label-gated on PRs (`run-real-decode`) + nightly; device primaries nightly; device cloud farm weekly; glasses weekly; embodied soak 24 h nightly / 72 h release-gated; cloud-relay label-gated + nightly. | |
| D21 | **Tiered release gates**: critical (always blocks), major (blocks unless waived by named approver), minor (warning). | Approvers: named per-phase; start with v2 tech lead (self) + 1–2 external reviewers per RFC. |

### D21 — Tier contents

- **Critical**: compositor hang/crash, audit log gap, embodied session state-machine violation, revoke >1 s, media escapes sandboxed surface.
- **Major**: p99 latency regression >20%, decode drop >1%, recording artifact unflushed, lip-sync drift >50 ms.
- **Minor**: unit test flake <1%, non-primary device lane failure, doc gap, perf regression <5%.

---

## Section E — Architecture

| ID | Decision | Notes |
|---|---|---|
| E22 | **Audio stack**: Opus codec, stereo channels, runtime-owned routing. Default output device: **operator-selected at first run, sticky**, changeable via config. | Spatial audio is phase 4 refinement. |
| E23 | **Upstream composition**: negotiable per device profile (e). Default: full local compositor for desktop/mobile, upstream-paired-host for glasses. Wire contract: pre-composited WebRTC frames + input event stream. **Operator policy enforcement at the compositor host**, not the endpoint. | Cloud compositing is post-v2. |
| E24 | **Shared worker pool** (N=2–4) with priority-based preemption. **In-process tokio tasks**, not subprocess isolation, with aggressive budget limits + watchdog. | |
| E25 | **Degradation ladder** (drop top-first under budget pressure): spatial audio → framerate → resolution → recording → cloud-relay → second stream → freeze+no-input → tear down media (keep session) → revoke embodied → disconnect. **Runtime-automatic on budget breach; operator-manual via revoke; no agent-initiated self-degrade.** | **Doctrine-destined**: the ladder ORDER and the "never-agent-initiated" rule land in `about/heart-and-soul/failure.md` (amendment) before cascading to RFC 0014 mechanism + validation-operations spec tests. |
| E26 | **State machines are defined in RFC 0014 (media) and RFC 0015 (embodied).** Packet records only their existence and approximate state set; the authoritative diagrams + transitions live in the RFCs. Implementation: `statig` crate + protobuf representation in session.proto. Mirrors v1 lease state machine pattern. | Approximate states — embodied: `REQUESTING → ACTIVE → ORPHANED → (ACTIVE \| EXPIRED)` plus transient `DEGRADED`; media: `ADMITTED → STREAMING → (DEGRADED \| PAUSED) → CLOSING → CLOSED` plus terminal `REVOKED`. |

---

## Section F — Prerequisites & sequencing

| ID | Decision | Notes |
|---|---|---|
| F27 | **Soft gate on v1 ship**: RFC/spec work on v2 can proceed in parallel; v2 *code* landing waits until v1 tagged `v1.0.0` on main + GitHub release with binary artifacts + closeout reconciliation report in `docs/reports/`. Once tagged, v2 code lands behind a `v2_preview` feature flag. | |
| F28 | **Absorb the existing bounded-ingress tranche into v2 phase 1**. No separate v1.1. Existing `openspec/specs/media-webrtc-bounded-ingress` + `media-webrtc-privacy-operator-policy` specs become references; v2's `media-plane` spec supersedes them explicitly at archive time with a pointer note. | **At archive time**, both superseded specs gain a top-of-file `SUPERSEDED-BY: v2-embodied-media-presence/media-plane` pointer block; the archive entry follows the existing `docs/reports/` closeout pattern. **Preservation audit**: every MUST/SHOULD requirement in both superseded specs gets an explicit line-by-line audit during archival. Preserved / superseded / dropped verdict recorded in phase-1 closeout report. |
| F29 | **Six RFCs must merge before bead creation** in their respective phases: RFC 0014 Media Plane Wire Protocol (phase 1, **≥2 external reviewers given fan-out across all later phases**), RFC 0015 Embodied Presence Contract (phase 2, ≥1 reviewer), RFC 0016 Device Profile Execution (phase 3, ≥1), RFC 0017 Recording and Audit (phase 4a, ≥1), RFC 0018 Cloud-Relay Trust Boundary (phase 4b, ≥1), RFC 0019 Audit Log Schema and Retention (phase 2, ≥1). Amendments: RFC 0002 (runtime-kernel) for media worker lifecycle, RFC 0005 (session-protocol) for embodied flag + media signaling (**must explicitly preserve `WidgetPublishResult.request_sequence` from `rust-widget-publish-load-harness` and any Layer 3 extension semantics added by `mcp-stress-testing`**), RFC 0008 (lease-governance) for C13's capability dialog + 7-day remember, RFC 0009 (policy arbitration) for C12 role-based operators. Each RFC is its own PR, merges before any implementation bead on that topic. Follows the existing `about/legends-and-lore/` pattern. | |
| F30 | **Doctrine merges before RFC** in each topic area. New files: `about/heart-and-soul/v2.md`, `embodied.md`, `media-doctrine.md`, `recording-ethics.md`. **Amendments** to existing doctrine: `v1.md` gets a "V2 Program" pointer section **plus explicit per-item supersession markers on the deferred-to-v2 items** (media plane, embodied presence level, WebRTC, GStreamer integration, clocked media/cue message class) — anchored by item heading rather than line number so markers survive future v1.md edits; `failure.md` amended with the E25 degradation ladder (doctrine surface); `presence.md` amended with A4 single-embodied-agent rule; `security.md` amended with E24 in-process-worker isolation posture + review. | Authorship: same approver pool as D21. |
| F31 | **Topology updates (lay-and-land pillar)**: `about/lay-and-land/components.md` gains entries for media-worker-pool (E24), audio-routing subsystem (E22), recording store, and audit-log store. `about/lay-and-land/data-flow.md` gains a media-plane data-flow diagram. These updates merge alongside the first implementation bead of their phase. | |
| F32 | **Engineering-bar updates (craft-and-care pillar)**: `about/craft-and-care/engineering-bar.md` §2 Performance Budgets gains the D18 numbers (glass-to-glass p50 ≤150 ms / p99 ≤400 ms, decode drop ≤0.5%, lip-sync drift ≤±40 ms, TTFF ≤500 ms). Review checklist gains real-decode-lane and device-lane items. D21 tier contents (critical/major/minor) are promoted to the bar as the v2 release gate. **Note co-tenancy**: `mcp-stress-testing` change also extends validation-framework Layer 3; v2's validation-operations spec must coordinate, not conflict. | |
| F33 | **Per-phase closeout report bead (v1 convention inherited as a hard rule)**. Each phase (1, 2, 3, and each of 4a/4b/4e/4f) MUST terminate in a `docs/reports/` closeout report following the v1 pattern (see `docs/reports/session_resource_upload_rfc0011_epic_report_20260417.md` as reference). The next phase's first implementation bead is blocked on the prior phase's closeout report merging to main. Closeout report content: scope delivered, deferred items, cross-pillar surfaces actually updated (heart-and-soul/legends-and-lore/lay-and-land/craft-and-care), open beads discovered-from, re-baseline verdict for the remaining calendar. | |

---

## Section G — Resourcing

| ID | Decision | Notes |
|---|---|---|
| G31 | **Rough calendar (pre-audit estimate, likely optimistic)**: phase 1 (2–3 mo → realistically 3–4), phase 2 (2 mo), phase 3 (3 mo, with 3.1 mobile overlapping late phase 2), phase 4 (4–6 mo → realistically 6–9 given legal-review serialization on 4a and voice-indicator doctrine on 4f). Total ≈ **12–17 months from v1 ship** (down-revised from 11–14). **MUST re-baseline after phase 1 closeout** — phase 2 kickoff bead is blocked on the re-baseline verdict. Hard-gating events: none named. | The re-baseline after phase 1 is a hard rule, not a suggestion. |
| G32 | **Solo operator (Tzeusy) + parallel LLM agent fleet** (Claude/Codex via Gas Town beads-coordinator infrastructure). V2 tech lead = Tzeusy. Phase leads do not apply — all phases run under the same human reviewer. Implementation parallelizes across agent workers; human review is the serialization bottleneck. **Dominant bottleneck modeled explicitly**: **~16 design-doc PRs** (6 new RFCs + 4 RFC amendments + 4 new doctrine files + 3 doctrine amendments + engineering-bar + lay-and-land updates), each requiring ≥1 external reviewer (RFC 0014 requires ≥2). Budget ~1 reviewer-week per RFC and ~0.5 reviewer-week per doctrine file; a naive serial schedule pushes the design-doc critical path to ~4 months by itself. Parallelize review by drafting multiple RFCs concurrently (agent-drafted, human-reviewed) and queueing external reviewers. | |
| G33 | **Procurement is owner = Tzeusy**, documented in sibling file `procurement.md`. | |

---

## Open items pending explicit confirmation

Three items were assumed but not explicitly answered. Confirm or override before finalizing the signoff:

1. **A2/A1 reconciliation**: bidirectional AV treated as v2-phase-4 scope given A5's voice-synthesis inclusion. Original A2 answer was v3 — confirm the flip.
2. **B6**: embodied-vs-resident distinguishing features assumed as (i)+(ii)+(iv)+(v) — durable ID, mandatory operator visibility, strict revocation, device-aware routing. Not: cert-based auth, forced media capability.
3. **E24 ↔ security.md**: respondent commits to reading `about/heart-and-soul/security.md` before phase 1 starts and deciding whether tze_hud's agent-isolation posture admits in-process tokio workers. This is **not a signoff-gate** — it is a phase-1 first-action. Two possible outcomes:
   - **Compatible**: record a security.md amendment in phase 1 that notes in-process worker compatibility rationale.
   - **Incompatible**: file a phase-1 blocker bead to pivot E24 to subprocess isolation (ripples through worker pool, priority preemption, budget watchdog); pause phase 1 implementation until the pivot lands.

If 1 and 2 are resolved and the respondent acknowledges 3, this packet is final and task 6.3 (generate v2 beads graph) is unblocked.

## Key risks called out explicitly

1. **RFC 0014 wire-protocol shape** is the highest-leverage irreversible decision in v2. Field numbers, message shape, and relationship to RFC 0005 session envelope cascade into every later phase (embodied, device profiles, recording, cloud-relay, bidirectional AV). Mitigated by ≥2-external-reviewer gate in F29.
2. **Cloud-relay SFU vendor lock-in** (C15). LiveKit-or-Cloudflare choice at 4b kickoff; self-host is post-v2. Migration between vendors is non-trivial.
3. **In-process tokio media workers** (E24). If security.md posture forces subprocess isolation, the worker-pool design redoes. Captured in open item 3b.

---

## Cross-pillar impact matrix

Decisions route across the 5-pillar knowledge architecture. Each decision with non-spec impact must have its doctrine / RFC / topology / engineering-bar surface authored *before* the corresponding implementation bead is created.

| Decision | heart-and-soul (doctrine) | legends-and-lore (RFC) | lay-and-land (topology) | craft-and-care (quality bar) | openspec (spec) |
|---|---|---|---|---|---|
| A1 | v2.md scope | — | — | — | — |
| A3 | — | RFC 0016 | components.md (glasses subsystem) | — | device-profiles |
| A4 | presence.md amendment | RFC 0015 | — | — | presence-orchestration |
| A5 (non-goals line 1) | v2.md | — | — | — | — |
| A5 (expansions) | — | RFC 0017 / 0018 | components.md (recording store, cloud-relay) | — | media-plane, presence-orchestration |
| B6 | embodied.md | RFC 0015 | — | — | presence-orchestration |
| B7 | — | RFC 0015 (+RFC 0005 amendment) | — | — | presence-orchestration |
| B8 | — | RFC 0015 | — | — | presence-orchestration |
| B9 | embodied.md (portability ethic) | RFC 0015 (export/import wire) | — | — | presence-orchestration + **new capability `identity-portability`** (owns key-material format, pairing ritual UX, operator flow for device migration) |
| B10 | — | RFC 0015 (+RFC 0005 amendment) | — | — | presence-orchestration |
| B11 | — | RFC 0014 | — | — | media-plane, presence-orchestration |
| C12 | — | RFC 0009 (policy arbitration) amendment | — | — | presence-orchestration + **new capability `identity-and-roles`** (owns role definitions, user directory schema, role-to-capability binding) |
| C13 | — | RFC 0008 (lease-governance) amendment | — | — | presence-orchestration |
| C14 | **recording-ethics.md** (mandatory indicator is a prime directive) | RFC 0017 | components.md (recording store) | — | media-plane |
| C15 | media-doctrine.md (trust boundary) | RFC 0018 | components.md (cloud-relay), data-flow.md | — | media-plane |
| C16 | — | RFC 0015 (revocation wire + numbers) | — | — | presence-orchestration |
| C17 | — | **RFC 0019 Audit Log Schema and Retention** | components.md (audit-log store) | engineering-bar.md (retention) | validation-operations |
| D18 | — | — | — | **engineering-bar.md §2 budgets** | validation-operations |
| D19 | — | — | — | engineering-bar.md (device lane) | validation-operations |
| D20 | — | — | — | engineering-bar.md (CI cadence) | validation-operations |
| D21 | — | — | — | **engineering-bar.md (tiered release gate)** | validation-operations |
| E22 | — | RFC 0014 (audio stack) | components.md (audio subsystem) | — | media-plane |
| E23 | — | RFC 0016 (upstream composition) | data-flow.md (glasses pipeline) | — | device-profiles |
| E24 | security.md amendment (isolation posture) | RFC 0014 (worker pool) | components.md (worker pool) | — | media-plane |
| E25 | **failure.md amendment** (ladder order + "never agent-initiated") | RFC 0014 (mechanism) | — | — | media-plane, validation-operations |
| E26 | — | RFC 0014 + RFC 0015 (state machines) | — | — | media-plane, presence-orchestration |

RFCs not otherwise itemized: RFC 0002 (runtime-kernel) receives an amendment for media worker lifecycle + degradation trigger authority.

### Spec → decision mapping (for bead graph authoring)

- `media-plane/spec.md` → A1, A2, A5 expansions 4a/4b, B10, C13, C14, C15, D18, D20, E22, E24, E25, F28, F29(RFC0014)
- `presence-orchestration/spec.md` → A4, B6, B7, B8, B9, B10, B11, C12, C16, C17, E26, F29(RFC0015)
- `device-profiles/spec.md` → A3, D19, E23, F29(RFC0016)
- `validation-operations/spec.md` → D18, D20, D21, F29(RFC0017, RFC0018, RFC0019)
- **new** `identity-and-roles/spec.md` → C12 role definitions, user directory schema, role-to-capability binding
- **new** `identity-portability/spec.md` → B9 key material format, pairing ritual, device-migration operator flow

---

## What happens after signoff

1. Confirm the three open items above (1, 2, 3a; 3b auto-files if needed).
2. Write the **six RFCs** (0014–0019) + 4 amendments (RFC 0002, 0005, 0008, 0009) and the four doctrine files (blocks per-phase bead creation per F29/F30).
3. Ship v1 → tag `v1.0.0` (blocks any v2 code landing per F27).
4. Run task 6.3: generate the v2 beads graph. Suggested epic structure: one epic per phase, with phase 4 fanned into **4a/4b/4e/4f** sub-epics (4c federation and 4d external transcoding are demoted to v3). Bead graph MUST include spec-scaffold beads for the two new capabilities `identity-and-roles` and `identity-portability` under `openspec/changes/v2-embodied-media-presence/specs/`.
5. Topology (F31) and engineering-bar (F32) updates are explicit beads within their phase's epic — not silent side effects.
6. Procure the GPU runner box (gates real-decode CI for phase 1) — see `procurement.md`.
7. Begin phase 1 work.
