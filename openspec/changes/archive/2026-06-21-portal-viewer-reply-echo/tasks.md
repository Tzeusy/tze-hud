# Tasks — Portal Viewer Reply Echo

This change documents already-shipped behavior (PR #967, bead `hud-o7h1r`) as a normative
requirement. Most tasks are reconciliation/verification against the implementation rather
than new implementation. The change is archived only after PR #967 merges to `main`.

## 1. Contract and review

- [x] 1.1 Validate this OpenSpec change with `openspec validate portal-viewer-reply-echo --strict` (passes: "Change 'portal-viewer-reply-echo' is valid")
- [x] 1.2 Confirm doctrine alignment: local-first interaction (`about/heart-and-soul/presence.md`), ambient attention / "not a notification engine" (`about/heart-and-soul/attention.md`, `vision.md`), and that the viewer turn is governed like any other transcript content (`privacy.md`) — echo is local-first (authored at submit time), does NOT bump unread-output count or escalate interruption class (asserted in submit tests), and carries the submission `content_classification` so it redacts like agent content
- [x] 1.3 Confirm the echo adds no new transport, RPC, or stream and does not change the existing bounded transactional submission contract — `append_viewer_echo` runs on the existing `submit_portal_input` path; submission still delivered transactionally to the adapter inbox, no new RPC/stream added

## 2. Verify implementation satisfies each scenario (PR #967)

- [x] 2.1 "accepted reply appears as a viewer turn" — `OutputKind::Viewer` + `append_viewer_echo` on accepted `submit_portal_input`; locked by `accepted_portal_input_echoes_viewer_reply_into_transcript` (tze_hud_projection)
- [x] 2.2 "viewer echo does not count as unread or escalate attention" — `append_viewer_echo` deliberately does NOT bump `unread_output_count`; asserted in the accepted/acknowledged submit tests
- [x] 2.3 "adapter cannot forge a viewer turn" — `parse_output_kind` rejects an adapter-supplied `viewer`; locked by `parse_output_kind_rejects_adapter_supplied_viewer` (tze_hud_runtime)
- [x] 2.4 "viewer echo redacts like transcript content" — echo carries the submission `content_classification`; covered by the collapsed-redacted-projection test (viewer "help" turn suppressed under redaction)
- [x] 2.5 "rejected submission is not echoed" — echo is gated on `result.is_ok()` in `submit_portal_input`; rejected path appends nothing and surfaces the legible rejection reason (`composer_feedback_line`, bead `hud-phdkd`)

## 3. Reconcile and close

- [x] 3.1 Note the deferred visual turn-differentiation work (alignment/role accent/attribution) on the promotion epic `hud-g1ena`; this requirement does not mandate pixel-level rendering
- [x] 3.2 After PR #967 merges, sync the delta to `openspec/specs/text-stream-portals/spec.md` (`/opsx:sync`) and archive (`/opsx:archive`)
