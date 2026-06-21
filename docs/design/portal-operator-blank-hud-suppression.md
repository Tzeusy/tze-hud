# Design: Operator Blank-HUD vs. Lease-Holding Agent Publish

**Issue:** hud-24i7o  
**Author:** design worker (agent/hud-24i7o)  
**Date:** 2026-06-21  
**Status:** Recommendation — awaiting owner ratification

---

## Problem Statement

`resolve_portal_host_tab` (
`crates/tze_hud_runtime/src/portal_projection_driver.rs:1058–1073`) was
introduced by the hud-obw3q fix to handle a real usability gap: a runtime
booted with a config whose default tab carries no widgets starts with
`scene.active_tab == None` (see `windowed/lifecycle.rs`). Before obw3q,
the portal driver's `CreatePortalTile` arm silently deferred (line 1199
pre-fix) when `tab_id == None`, dropping the coalesced update and leaving
an accepted publish invisible.

The fix is correct for the boot case. The new function activates the
lowest-`display_order` tab (or creates a default "Main" tab) to ensure
the first cooperative publish always renders.

**The ambiguity:** `scene.active_tab == None` now carries two distinct
operator intents that the driver cannot distinguish:

| Case | Intent | Correct driver response |
|------|--------|------------------------|
| (a) Boot: default tab has no widgets | No tab has been active yet | Auto-activate — let the publish render |
| (b) Operator deliberately blanked the HUD | All tabs were deleted or no active tab should be shown | Defer — respect the operator's choice |

Currently, case (b) has no dedicated representation. Both cases are
`active_tab == None`. An agent that holds a valid projection lease and
publishes will trigger auto-activation in both cases, potentially
overriding an explicit operator intent.

There is presently **no operator-side affordance** to suppress agent-initiated
tab activation. This document analyzes whether one is warranted for v1.

---

## Doctrine Constraints

### Screen is sovereign (highest principle)

> "**Screen is sovereign.** The runtime owns pixels, timing, composition,
> permissions, arbitration. Models request via leases with TTL, capability
> scopes, and revocation semantics."
— `CLAUDE.md` / `about/heart-and-soul/architecture.md`

This is a hard constraint: operators can always override agent content.
A mechanism that lets an agent auto-activate a tab against explicit operator
intent would violate this principle.

### Human override is Level 0 in the arbitration stack

> "Level 0 — HUMAN OVERRIDE [HIGHEST]: Dismiss, safe mode, freeze, mute.
> Local, instant, cannot be intercepted, delayed, or vetoed."
— RFC 0009 §1.1 (`about/legends-and-lore/rfcs/0009-policy-arbitration.md`)

> "The human is always the ultimate authority. No agent, regardless of trust
> level or capability scope, can prevent the human from: dismissing any tile
> or overlay, revoking any lease, terminating any agent session, muting any
> media stream, freezing the scene, entering a 'safe mode' that disconnects
> all agents. These overrides are handled locally by the runtime, not routed
> through an agent. They cannot be intercepted, delayed, or vetoed."
— `about/heart-and-soul/security.md`, §"Human override"

### Agent presence requires active lease; tab activation is a separate capability

RFC 0008 §1.2 (`about/legends-and-lore/rfcs/0008-lease-governance.md`):
> "`CreateTab`, `RemoveTab`: No — tab operations require `manage_tabs`
> capability, not a surface lease."

Tab management is not a lease permission. An agent's projection lease
authorizes it to publish text content to a portal tile; it does not
authorize it to activate or create tabs. The hud-obw3q fix embedded tab
activation inside the portal driver's internal runtime lease
(`PORTAL_DRIVER_NAMESPACE`) as a convenience, not as an agent-capability
grant.

### RFC 0013 — text-stream portals require governance

> "Portal surfaces remain lease-bound or runtime-managed; no side channel
> bypass."
— RFC 0013 §"Design Requirements Satisfied"
(`about/legends-and-lore/rfcs/0013-text-stream-portals.md`)

The cooperative projection path must not become a route around operator
controls. Auto-activation on behalf of a publishing agent is fine for the
boot case; it must not be an agent bypass of operator intent.

### Safe mode is the existing mechanism for operator suppression

RFC 0008 §3.4 defines safe mode: all `ACTIVE` leases → `SUSPENDED`,
agent mutations blocked, tiles frozen with staleness badge. This is the
doctrinal "operator stop" for agent content. RFC 0009 Level 0 / Level 1
govern it. The human can enter and exit safe mode without agent cooperation.

---

## Code Reality

### What state is available today

`scene.active_tab: Option<SceneId>` (
`crates/tze_hud_scene/src/types.rs:2710`) is the only tab-selection state.
It is:
- `None` at boot if the default tab has no widgets (lifecycle.rs:911 area)
- `Some(id)` after the first tab is activated or a non-widget-less tab exists
- `None` again after all tabs are deleted (tabs.rs:130–136, fallback logic)

There is **no** `HudSuppressed`, `hud_mode`, `operator_blank`, or similar
field anywhere in `SceneGraph` or the runtime.

### How `resolve_portal_host_tab` is called

In `drain_inner` (portal_projection_driver.rs:1187–1214):
```rust
let resolved_tab = tab_id.or_else(|| Self::resolve_portal_host_tab(scene));
```
`tab_id` is `scene.active_tab` passed in from the drain call site. When
that is `None`, `resolve_portal_host_tab` fires unconditionally.

### Existing operator controls for portal suppression

The operator today can:
1. **Operator cleanup** (MCP `portal_projection_cleanup` with
   `cleanup_authority = "operator"`): purges a specific projection session.
   This removes the session's coalescer entry and detaches the projection
   so further publishes are rejected. But it does not persist across a
   new `portal_projection_attach`.
2. **Safe mode** (RFC 0008): suspends all leases, blocks all agent
   mutations system-wide. Full and reversible, but coarse-grained (affects
   all agents, not just portals).
3. **Delete tab** (`delete_tab` mutation via gRPC): removes the active tab.
   If no other tab exists, `active_tab` falls to `None`. The next agent
   publish then triggers `resolve_portal_host_tab` and re-activates/creates
   a tab — defeating the operator's intent.

There is currently **no fine-grained "suppress agent tab activation" knob**
that persists across attach/publish cycles.

---

## Design Options

### Option A: Explicit `HudSuppressed` / `operator_tab_policy` scene state

**Mechanics:**  
Add a `operator_tab_activation_suppressed: bool` (or `HudTabPolicy` enum)
to `SceneGraph`. A new operator MCP call (e.g., `set_hud_tab_policy(mode:
"normal"|"suppressed")`) sets this flag. `resolve_portal_host_tab` checks
it at the top of the function and returns `None` (deferring the drain)
when suppressed.

```rust
// resolve_portal_host_tab — proposed guard:
if scene.operator_tab_activation_suppressed {
    tracing::debug!("operator suppressed tab activation — deferring portal tile creation");
    return None;
}
```

Revocation: the operator must explicitly lift suppression. An agent publish
does not lift it.

**Doctrine fit:** Strong. This is precisely what Level 0 (Human Override)
requires: an operator choice that an agent cannot override. The field
lives in the scene graph (the single source of truth), making it
observable in scene diffs and snapshots.

**Implementation cost:** Low–moderate. Changes:
- `crates/tze_hud_scene/src/graph/` — add `operator_tab_activation_suppressed` field to `SceneGraph`; serialize/deserialize; expose via mutation or admin RPC
- `crates/tze_hud_runtime/src/portal_projection_driver.rs:1058` — two-line guard
- New MCP resident tool (or gRPC admin call) to set/clear the flag
- Scene snapshot diff tracking (already automatic via version bump)
- Tests for suppressed state + drain behavior

**Blast radius:** Contained. The field is a new boolean; existing paths
are unchanged when it is `false` (the default).

**UX:** Clear and explicit. Operator sets intent; agent respects it.
The HUD stays blank until the operator lifts suppression. Durable across
attach/re-attach cycles.

**Revocation semantics (RFC 0008 alignment):** The operator's suppress
flag is not a lease state — it is a scene-level policy. It does not
interact with lease TTL, suspension, or renewal. It is lifted only by
explicit operator action, matching the "cannot be intercepted, delayed,
or vetoed" requirement for Level 0 overrides.

---

### Option B: Capability gate — require `portal:auto_activate_tab` capability

**Mechanics:**  
Add a new capability `portal:auto_activate_tab` to the capability
vocabulary (RFC 0008 Amendment / RFC 0009 §8). The portal driver's
internal runtime lease (`PORTAL_DRIVER_NAMESPACE`) starts without this
capability. The operator grants it at session admission or via a runtime
lease renegotiation. Without it, `resolve_portal_host_tab` refuses to
auto-activate.

**Doctrine fit:** Moderate. RFC 0008 §1.2 gates tab operations on
`manage_tabs`; by analogy, auto-activation is a tab operation. However,
the portal driver uses a runtime-internal lease, not the projecting
agent's session lease, so the gate is applied in the wrong place. The
projecting agent (the external session) holds a `portal_projection_*`
session; the runtime driver holds `PORTAL_DRIVER_NAMESPACE`. Threading
the projecting agent's capability grants into the driver's tab-activation
decision adds coupling without proportionate clarity.

**Implementation cost:** High. Requires:
- New capability string in the vocabulary
- `ensure_driver_lease` must now accept a parameterized capability set or
  check the projecting agent's session capabilities — a cross-layer coupling
  that does not exist today
- Session admission paths must thread this new grant
- Operator UX for "withhold this capability" is not designed

**UX:** Poor for operators. Capabilities are admission-time grants, not
runtime choices. An operator who wants to "blank the HUD right now" has
no natural path to withdraw a capability mid-session without revoking
the session entirely. This is a session-management knob, not an
immediate-override knob, so it violates the "local, instant" requirement
of Level 0.

**Verdict:** Does not fit the use case. Capabilities gate what agents
_are allowed to do in principle_, not what the operator _wants right now_.

---

### Option C: Status quo + documented limitation (no code change)

**Mechanics:**  
Accept the current behavior. Add a code comment to `resolve_portal_host_tab`
and a note to the relevant docs documenting the known ambiguity. Recognize
that "operator deliberately blanked HUD by deleting all tabs" is not a
real operator workflow today and that safe mode covers the coarse-grained
suppression case.

**Doctrine fit:** Weak for the principle, but the gap is latent not active.
No current operator affordance creates a deliberate blank-HUD state that
survives a cooperative publish. The most natural operator blank (delete all
tabs) is destructive and unusual. The existing safe mode covers the "stop
all agents now" need.

**Implementation cost:** Zero (modulo comment).

**UX:** Leaves a documented gap. If an operator deletes all tabs expecting
a blank HUD, the next agent publish re-populates one — unintuitive.

**Risk:** Bakes in the wrong behavior if an explicit blank-HUD operator
workflow is later designed. A flag added retroactively is a behavior
change; documenting it now and deferring the wire is lower cost.

**Verdict:** Acceptable only if the operator blank-HUD workflow is
genuinely unneeded in v1 and safe mode is sufficient.

---

### Option D: Runtime-local suppress flag (non-scene, portal driver owned)

**Mechanics:**  
Instead of adding state to `SceneGraph`, store a `bool
suppress_agent_tab_activation` on `InProcessPortalDriver` itself. A new
MCP resident call sets/clears it. The flag is checked in the
`CreatePortalTile` arm and in `resolve_portal_host_tab`. It is not
scene-graph state and does not survive a runtime restart.

**Doctrine fit:** Partial. The operator can suppress activations, but the
state is not visible in scene snapshots/diffs, not observable by the
gRPC event stream, and not durable across restarts. It lives in the
driver, not the runtime's canonical state.

**Implementation cost:** Lower than Option A (no scene graph schema
change), but creates a hidden piece of state that cannot be diffed or
replayed.

**UX:** Operator gets immediate control. However, the invisible state is
a maintenance hazard — any scene restore, snapshot replay, or headless
test that does not explicitly set the flag will see different behavior.

**Verdict:** A workable short-term expedient, but it violates the
"single source of truth" scene graph principle and defers the right
design.

---

## Recommendation

**For v1: Option C with a documented limitation + proactive comment
hardening, and a filed follow-up bead for Option A implementation when
a genuine blank-HUD operator UX is designed.**

**Rationale:**

1. **The use case is latent, not active.** No current operator workflow
   creates a deliberate blank-HUD state that `resolve_portal_host_tab`
   can violate. The only real-world `active_tab == None` case observed
   was the obw3q boot case, which the fix correctly addresses.

2. **Safe mode is the right v1 coarse suppression tool.** An operator who
   wants to stop all agent output has safe mode (RFC 0008 §3.4). It is
   doctrinal, implemented, and tested. A fine-grained portal-specific
   tab-activation suppress flag duplicates part of that control without
   its full governance semantics.

3. **Premature implementation adds scope without demand.** V1 is focused
   on a high-quality Windows-first runtime (see `about/heart-and-soul/v1.md`,
   "Single-Windows Refocus"). Adding a new operator affordance — MCP tool,
   scene flag, serialization, test coverage — requires demand that does not
   yet exist.

4. **Option A is the right shape if the demand emerges.** The design above
   is correct: a scene-level `operator_tab_activation_suppressed: bool`,
   set via a new operator MCP call, checked at the top of
   `resolve_portal_host_tab`. It respects doctrine, has low blast radius,
   and is clearly reversible. It should be implemented the moment an
   operator blank-HUD UX is designed and the use case is confirmed.

5. **The current code needs a comment.** The existing comment on
   `resolve_portal_host_tab` explains the obw3q rationale but does not
   document the limitation. A follow-up commit should add a `# KNOWN LIMITATION`
   note (without changing behavior) to make the ambiguity visible to
   future maintainers.

### Doctrinal caveat

The recommendation accepts a bounded doctrine gap: an agent publish can
auto-activate a tab against an operator's `delete_tab` intent. This is
tolerable only because the path has no natural operator entrypoint today
and because safe mode covers the critical case. If an operator blank-HUD
workflow is designed (e.g., a "blank display" UI control that clears all
tabs and expects the HUD to stay blank), **Option A becomes mandatory
before that UI ships.**

**This recommendation requires owner ratification.** The owner makes the
final call on whether the current tolerance is acceptable or whether
Option A should be implemented proactively.

---

## Implementation Outline (if Option A is approved)

Files touched (minimal):

1. **`crates/tze_hud_scene/src/graph/` — add scene field**
   - Add `operator_tab_activation_suppressed: bool` to `SceneGraph`
     (default `false`; serde with `#[serde(default)]`)
   - Expose a mutation: `SetOperatorTabPolicy { suppressed: bool }` in
     `mutation.rs` (or admin-only gRPC path — see note below)
   - Bump `version` when the field changes so scene diffs catch it

2. **`crates/tze_hud_runtime/src/portal_projection_driver.rs:1058`**
   - Two-line guard at the top of `resolve_portal_host_tab`:
     ```rust
     if scene.operator_tab_activation_suppressed {
         return None;  // Operator suppressed tab activation — defer
     }
     ```

3. **MCP resident tool or gRPC admin call**
   - New MCP resident tool: `set_hud_tab_policy(suppressed: bool)` (requires
     `operator_authority` or a new `manage_display_policy` capability)
   - Alternatively, an admin gRPC message avoids adding a new MCP surface

4. **Tests**
   - `portal_drain_operator_suppressed_defers_create_portal_tile`: assert
     drain defers (does not activate tab) when flag is set
   - `portal_drain_operator_suppress_lifted_creates_tile`: assert drain
     proceeds after flag is cleared
   - Regression guard: existing `drain_with_no_active_tab_activates_tab_and_paints`
     must still pass (flag is `false` by default)

Total blast radius: ~3 files, ~30–50 lines of non-test code, ~50–80 lines
of test code. No RFC amendment required (this is a new capability, not a
protocol change — unless the MCP surface is added, which requires a tools.rs
doc-comment update per hud-5w1pb conventions).
