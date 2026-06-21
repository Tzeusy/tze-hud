## Why

A text stream portal is, per `about/heart-and-soul/vision.md`, "a persistent on-screen
portal where a person can converse with another human through a chat transport, or with
an LLM through an agent session." Conversation is two-way. But the `text-stream-portals`
spec (`openspec/specs/text-stream-portals/spec.md`) only specifies one direction of the
transcript: agent-authored output (`OutputKind` = assistant/tool/status/error/other) plus
a **bounded input submission** that the adapter maps into its semantic inbox
(§Cooperative Projection Input Mapping). It is silent on what the viewer *sees* after they
submit a reply.

The shipped behavior before this change matched that silence: a submitted reply routed
into the agent's pending-input queue and **nothing appeared on screen** — the viewer's own
words vanished. That is structurally one-sided and reads as "the message didn't send,"
which `about/heart-and-soul/vision.md` explicitly warns against ("not a chatbot with a
screen" forbids reducing the surface to text, but a portal that *serves* presence must
still behave coherently as a conversation).

PR #967 closed the implementation gap: `OutputKind::Viewer`
(`crates/tze_hud_projection/src/contract.rs`) is a runtime-authored turn kind appended by
`append_viewer_echo` on an accepted `submit_portal_input`
(`crates/tze_hud_projection/src/authority.rs`), and `parse_output_kind`
(`crates/tze_hud_runtime/src/portal_projection_driver.rs`) rejects an adapter-supplied
`viewer` so a turn cannot be forged. This is **code ahead of spec**: the behavior is
implemented and tested but no normative requirement covers it. This change adds that
requirement so the contract — including the governance, attention, and anti-forgery
properties — is captured and testable.

## What Changes

One ADDED requirement on `text-stream-portals`:

- **Viewer Reply Echo** — when a viewer submits a reply that the portal accepts, the
  runtime echoes the submitted text into the retained transcript as a runtime-authored,
  kind-distinct **viewer turn**, so the two-way conversation is visible. The viewer turn:
  cannot be forged by an adapter (publishing the viewer kind through the output contract is
  rejected); carries the submission's content classification and obeys the same redaction,
  safe-mode, freeze, and Bounded Transcript Viewport rules as agent output; does **not**
  increment the unread-output count or escalate interruption class (the viewer has already
  seen their own message — consistent with Ambient Portal Attention Defaults); does not
  alter the existing transactional submission contract (the text still reaches the adapter
  inbox per Cooperative Projection Input Mapping); and is only echoed for an **accepted**
  submission (a rejected submission is not echoed).

## What Does Not Change

- No new transport, RPC, or stream: the echo is runtime-authored presentation of an
  already-accepted submission on the existing primary session stream.
- No change to the submission contract: submit remains a bounded transactional action and
  the submitted text still maps to the adapter's semantic inbox per the existing
  Cooperative Projection Input Mapping requirement.
- No scene-graph history: viewer turns are ordinary transcript units within the existing
  Bounded Transcript Viewport budget.
- No relaxation of governance: the viewer turn is not automatically safe because it is the
  viewer's own text — it redacts under the same policy as agent content.

## Non-Goals

- **Visual turn differentiation** (alignment, role accent, sender attribution, bubble
  layout) — this requirement establishes that viewer turns are first-class, kind-distinct
  units; the *pixel-level* rendering of agent-vs-viewer turns is token/component-profile
  styling folded into the promotion epic (`hud-g1ena`), not mandated here.
- Per-message timestamps, delivery/seen ticks, unread divider, jump-to-latest — those are
  separate chat-grade affordances tracked independently (audit backlog `hud-0yrix`); this
  change is only the viewer-turn echo contract.
- Editing or recalling a submitted reply — submission stays a one-shot bounded transactional
  action.
