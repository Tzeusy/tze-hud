---
name: heart-and-soul
description: >
  CRITICAL — This skill provides the foundational doctrine for tze_hud, an agent-native
  presence engine. Every agent working in this repository MUST consult relevant soul
  doctrine before making architectural decisions, writing code, designing APIs, creating
  tests, or proposing features. The about/heart-and-soul/ directory contains the project's
  prime directives: what the system is, what it is not, how it works, how it is tested,
  and what v1 ships. To avoid context overload, selectively load ONLY the documents
  relevant to your current task — do not load all 10 files at once. Use this skill
  proactively at the start of any substantive work session, when making design decisions,
  when unsure about project conventions, or when the task touches architecture, testing,
  security, presence model, failure handling, privacy, mobile, development workflow, or
  v1 scope.
---

# tze_hud Heart and Soul — Project Doctrine

The `about/heart-and-soul/` directory contains the prime directives of tze_hud. These are not documentation — they are doctrine. They define the principles the code must embody.

**You MUST consult relevant soul files before:**
- Making any architectural or design decision
- Writing new modules, traits, or public APIs
- Adding or modifying test infrastructure
- Proposing features or scope changes
- Reviewing code for alignment with project values

**You MUST NOT load all files at once.** Select only what your current task requires. Each file is self-contained.

## Document index — select by relevance

Read `about/heart-and-soul/README.md` for the full reading order. Below is the selection guide:

### Always relevant (skim first on any task)

| File | Read when... | Key content |
|------|-------------|-------------|
| `about/heart-and-soul/vision.md` | Starting any session, onboarding, scope questions | Core thesis, performance-is-product, **non-goals** (what tze_hud is NOT) |
| `about/heart-and-soul/v1.md` | Implementing anything, scoping features | What v1 ships, what it defers, success criteria, platform targets |

### Select by task domain

| File | Read when... | Key content |
|------|-------------|-------------|
| `about/heart-and-soul/architecture.md` | Protocol work, transport decisions, rendering, media, window model | Three protocol planes (MCP/gRPC/WebRTC), message classes, timing model, tech stack, **overlay/HUD click-through model**, anti-patterns |
| `about/heart-and-soul/presence.md` | Scene model, tiles, tabs, leases, multi-agent, interaction | Tab/tile/node hierarchy, three presence levels, lease governance, multi-agent coordination, orchestration, input model |
| `about/heart-and-soul/security.md` | Auth, capabilities, isolation, resource budgets, trust model | Trust gradient (guest→resident→embodied), capability scopes, agent isolation, resource governance, human override |
| `about/heart-and-soul/privacy.md` | Household display, viewer context, content visibility, interruptions | Viewer classes, content classification tiers, redaction behavior, interruption classes, quiet hours, multi-viewer policy |
| `about/heart-and-soul/attention.md` | Attention philosophy, attention budget, anti-patterns | Why presence ≠ attention capture, attention budget as runtime constraint, anti-patterns (notification spam, escalation creep, engagement dark patterns), ambient-over-interruptive principle |
| `about/heart-and-soul/failure.md` | Error handling, recovery, degradation, reconnection | Agent crash/slow/noisy/misbehave handling, scene persistence, reconnection contract, degradation ladder |
| `about/heart-and-soul/mobile.md` | Mobile/glasses targets, capability negotiation, degradation | Two deployment profiles, mobile design principles, protocol requirements, upstream composition |
| `about/heart-and-soul/validation.md` | Testing, benchmarks, CI, telemetry, LLM dev loop | Testing doctrine (spirit > letter), five validation layers, **split latency budgets**, calibration vector, fuzzing/chaos, developer visibility artifacts, test scene registry |
| `about/heart-and-soul/development.md` | Workflow, specs, task management, execution process | OpenSpec lifecycle, Beads task tracking, Coordinator/Worker/PR-Reviewer roles, 15 development principles |

## How to load

Read files directly from the `about/heart-and-soul/` directory:

```
Read about/heart-and-soul/vision.md        # always a good starting point
Read about/heart-and-soul/v1.md            # to understand what's in scope
Read about/heart-and-soul/architecture.md  # if touching protocols or rendering
```

For a quick orientation without loading full files, read just `about/heart-and-soul/README.md` — it has the one-line summary per file and reading order.

## Rules from the soul

These rules are absolute and non-negotiable. They appear in the soul files but are repeated here because violating them is a project-level failure:

1. **LLMs must never sit in the frame loop.** Models drive the scene; the runtime composits.
2. **The screen is sovereign.** The runtime owns pixels, timing, composition, permissions.
3. **Arrival time ≠ presentation time.** All payloads carry timing semantics.
4. **Local feedback first.** Touch/interaction acknowledgement is instant and local.
5. **Presence requires governance.** Agents hold leases with TTL, scopes, and revocation.
6. **Tests measure spirit, not letter.** The north star is consistent, deterministic, performant functionality with clean APIs — not green checkmarks.
7. **The human can always override.** Dismiss, mute, revoke, freeze, safe mode — always available, never interceptable.
