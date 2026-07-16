# Production Command Input Adapter Design

## Decision

Wire the existing `tze_hud_input::CommandProcessor` into the windowed runtime's
keyboard path. RFC 0004 defines keyboard as one concrete binding for the
platform-neutral command vocabulary; therefore keyboard is the first
production pointer-free source. This closes the current library/test-only seam
without adding a glasses-, D-pad-, or portal-specific runtime API.

The rejected alternatives are a programmatic-only injection hook, which would
remain unreachable from production input, and a portal-only adapter, which
would fork input semantics by surface. Future platform adapters may produce the
same `RawCommandEvent` with `Dpad`, `Voice`, `RemoteClicker`, or `RotaryDial`
sources without changing focus, routing, or protocol code.

## Data Flow and Precedence

`WindowEvent::KeyboardInput` continues to normalize physical and logical keys
into `RawKeyDownEvent`. The production keyboard drain classifies RFC 0004's
default bindings into `RawCommandEvent`, drives focus cycling for
`NAVIGATE_NEXT`/`NAVIGATE_PREV`, then passes the resulting focus owner and live
scene through `CommandProcessor`. The returned `CommandDispatch` is converted
to the existing protobuf `CommandInputEvent` and broadcast to the focused
owner's `INPUT_EVENTS` subscription.

Input precedence remains: safe-mode and shell-reserved shortcuts, portal
resize shortcuts, focus traversal, composer editing, then focused command or
raw-key dispatch. Composer Enter/Space/Escape and caret keys remain local draft
operations. Mouse input is untouched. Existing portal controls retain their
earlier pointer-equivalent compatibility action; the generic command adapter
serves ordinary focused scene elements without changing portal semantics.

## Local Feedback and Failure Handling

`ACTIVATE` uses `CommandProcessor`'s existing local pressed-state update before
agent delivery. The runtime records the activated node and clears its pressed
state on the matching activation-key release so feedback cannot stick. If the
shared runtime or scene lock exceeds the existing bounded interaction budget,
the raw keyboard event is deferred through the established FIFO retry queue;
it is never silently converted into a raw key out of order. Missing focus,
chrome focus, or a vanished tile produces no agent dispatch, matching RFC 0004
routing rules.

## Verification

Headless event-loop tests drive the real `WinitApp` keyboard drain with a live
scene and broadcast channel. They prove a keyboard `ACTIVATE` reaches the
focused agent as a transactional `CommandInputEvent` with the correct IDs and
sets local pressed state, and that Tab produces focus movement followed by a
`NAVIGATE_NEXT` command. Focused composer tests remain regression coverage for
precedence. The implementation must also pass the input/runtime focused suites,
workspace check, full all-target clippy, and integration compilation.

Traceability: RFC 0004 §§1.3, 5.7, 8.2, 10; `openspec/specs/input-model/spec.md`
requirements “Focus Cycling”, “Command Input Model”, “ACTIVATE Local Feedback”,
and “Pointer-Free Navigation”.
