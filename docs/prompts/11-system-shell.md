# Epic 11: System Shell

> **Dependencies:** Epic 0 (test infrastructure), Epic 2 (compositor/chrome layer), Epic 4 (lease suspension), Epic 8 (policy stack)
> **Depended on by:** Epic 12 (integration tests)
> **Primary spec:** `openspec/changes/v1-mvp-standards/specs/system-shell/spec.md`
> **Secondary specs:** `lease-governance/spec.md` (suspension), `policy-arbitration/spec.md` (Level 0 overrides), `configuration/spec.md` (privacy/redaction)

## Prompt

Create a `/beads-writer` epic for **system shell** — the chrome layer that provides human override controls, safe mode, freeze, privacy redaction, and disconnection badges. The shell is sovereign: agents cannot occlude chrome, steal focus from chrome, or prevent human overrides.

### Context

The system shell owns the topmost render layer (chrome) and all human override state. It is the exclusive owner of freeze/safe-mode interaction — policy-arbitration evaluates but does not transition shell state. The existing compositor has a layered surface stack (background, content, chrome) but the shell behavior logic is not yet implemented.

### Epic structure

Create an epic with **5 implementation beads**:

#### 1. Chrome layer sovereignty (depends on Epic 2 compositor)
Implement chrome layer rules per `system-shell/spec.md` Requirement: Chrome Layer Sovereignty.
- Chrome layer always rendered on top of agent content
- Agents cannot create tiles in chrome layer
- Chrome includes: tab bar, system indicators, override controls
- Hit-test: chrome always wins (checked before agent tiles)
- **Acceptance:** Layer 1 pixel tests confirm chrome always visible. Agent tile at max z-order still below chrome. Chrome hit-test priority verified.
- **Spec refs:** `system-shell/spec.md` Requirement: Chrome Layer Sovereignty

#### 2. Safe mode: suspend and resume (depends on #1, Epic 4 lease suspension)
Implement safe mode per `system-shell/spec.md` Requirement: Safe Mode Protocol.
- Safe mode suspends all agent leases (NOT revoke — identity preserved)
- Triggered by: Level 1 Safety (automatic, GPU failure/scene corruption) or Level 0 Human Override (Ctrl+Shift+Escape)
- During safe mode: mutations rejected, chrome overlay displayed, SessionSuspended sent
- Exit: explicit viewer action (click Resume, Enter/Space, or Ctrl+Shift+Escape toggle)
- On exit: leases restored to ACTIVE, SessionResumed sent, TTL clocks resume with suspension excluded
- Safe mode cancels active freeze (shell state invariant: safe_mode=true implies freeze_active=false)
- **Acceptance:** Safe mode suspends leases correctly. Mutations rejected during safe mode. Exit restores leases. TTL pause verified. Freeze cancelled on safe mode entry. `policy_matrix_basic` test scene passes.
- **Spec refs:** `system-shell/spec.md` Requirement: Safe Mode Protocol, `lease-governance/spec.md` Requirement: TTL Pause During Suspension

#### 3. Freeze semantics (depends on #1)
Implement freeze per `system-shell/spec.md` Requirement: Freeze Semantics.
- Freeze queues agent mutations (does not reject them)
- Pauses resource budgets and degradation ladder advancement
- Defers attention signals
- Auto-unfreeze on configurable timeout (default 5 minutes)
- Freeze attempted during safe mode is ignored
- **Acceptance:** Mutations queued during freeze, applied on unfreeze. Budget timers paused. Auto-unfreeze after timeout. Freeze ignored during safe mode.
- **Spec refs:** `system-shell/spec.md` Requirement: Freeze Semantics

#### 4. Privacy redaction (depends on #1, Epic 7 privacy config)
Implement redaction per `system-shell/spec.md` Requirement: Redaction Placeholder.
- Viewer context determines content visibility (owner/household/guest/unknown/nobody)
- Content exceeding viewer access replaced with neutral pattern (redaction_style: pattern|blank only)
- No agent name, content hint, or icon shown during redaction
- Layout preserved (prevents information leak about content shape)
- Interactive affordances (hit regions) disabled while redacted
- **Acceptance:** `privacy_redaction_mode` test scene passes. Redacted tiles show pattern/blank, no agent content. Layout dimensions preserved. Hit regions disabled.
- **Spec refs:** `system-shell/spec.md` Requirement: Redaction Placeholder, `configuration/spec.md` Requirement: Privacy Configuration

#### 5. Disconnection badges and backpressure signals (depends on #1)
Implement status indicators per `system-shell/spec.md` Requirement: Disconnection Badges, Requirement: Backpressure Signals.
- Disconnection: badge overlay on tiles owned by disconnected agents
- Badge appears within 1 frame of disconnect detection
- Badge clears within 1 frame of reconnection
- Backpressure: generic visual signal when agent's queue is under pressure
- Agents not told the scene is frozen (intentionally generic signals)
- **Acceptance:** Badge appears on disconnect. Badge clears on reconnect. Backpressure signal visible under load. Timing assertions (within 1 frame).
- **Spec refs:** `system-shell/spec.md` Requirement: Disconnection Badges, Requirement: Backpressure Signals

### Requirements for every sub-bead

**Every sub-bead description MUST include:**
1. **Explicit spec links** — cite `system-shell/spec.md` requirement names and line numbers
2. **WHEN/THEN scenarios** — reference exact spec scenarios
3. **Acceptance criteria** — which test scenes must pass (Layer 0 invariants + Layer 1 pixels)
4. **Crate/file location** — shell state machine module in runtime crate
5. **Ownership rule** — shell state transitions are the ONLY owner of freeze/safe-mode interaction; policy evaluates but does not transition

### Dependency chain

```
Epics 0+2+4+8 ──→ #1 Chrome Sovereignty ──→ #2 Safe Mode
                                         ──→ #3 Freeze
                                         ──→ #4 Privacy Redaction
                                         ──→ #5 Badges/Backpressure
```
