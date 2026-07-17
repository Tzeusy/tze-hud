# Pending-input piggyback token-footprint candidate v2

Status: **candidate, unapproved**. This packet records the isolated
`hud-vconx` measurement only. It does not replace the owner-approved v1
baseline, does not alter the fail-closed v1 checker, and carries no owner
decision reference.

## Scope and repeatability

The calibration now has two explicit modes:

- Default `legacy-v1` preserves the approved fixture and explicit
  `get_pending_input` call.
- `piggyback-candidate-v2` injects the same deterministic HUD input before a
  question publish, consumes it from that publish response, and omits the
  explicit poll.

Two complete candidate runs produced byte-identical generated JSON with SHA-256
`00f7fa6ff98dc2c87b61501a1c7e83739eff2add0266da1b6f636f61b64a8944`.
The generated v1 measurement remained semantically identical to
`token_footprint_candidate_v1.json` after excluding that approved packet's
`approval` object, and `check_token_footprint.py` reported `passed` against the
v1 baseline.

The committed candidate values are in
`scripts/ci/token_footprint_piggyback_candidate_v2.json`. Its approval status
is intentionally `candidate_unapproved`.

## Exact candidate identity

- Tokenizer: `tiktoken-rs` `0.12.0`, `o200k_base`.
- Vocabulary SHA-256: `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`.
- Fixture fingerprint: `blake3:9d1eadc5f141a0cd6b57b05fabc2915b9ef106f819a0f0e68297baa7b3c7da15`.
- Portal flow version: `2`.
- Portal flow fingerprint: `blake3:337fd0f648841c3d0aed2eb3e2f51bf36802f91904befd81527563f5599c66c7`.
- Zone and widget flows retain v1 version/fingerprints.

## Before and after

The canonical conversational turn excludes the one-time `attach` and includes
publish, input delivery, and acknowledgement.

| Version | Operations | Bytes | Tokens |
|---|---|---:|---:|
| Approved v1 | `publish` + `get_pending_input` + `acknowledge_input` | 1,886 | 502 |
| Candidate v2 | `publish` + `acknowledge_input` | 1,423 | 367 |
| Delta | one poll removed | -463 | -135 |
| Relative reduction | — | 24.55% | 26.89% |

The candidate's question-publish response carries the normal delivered input
payload, so the measured saving is **26.89%**, not the issue's approximate
42% estimate. That estimate should be confirmed or explicitly revised by the
owner before any baseline evolution.

### Reconciliation of the approximate 42% estimate

The candidate canonical turn **does not issue**
`portal_projection_get_pending_input`: its deterministic trace has only
`attach`, `publish`, and `acknowledge_input`, and the candidate-mode driver
test rejects an explicit poll. The 42% estimate is therefore not being missed
by a hidden legacy operation. It is the gross removal of the v1 poll:

- Removed v1 poll: 784 bytes / 210 tokens, which is 41.83% of the 502-token
  v1 conversational turn.
- Required v2 delivery payload in the already-existing publish response:
  +321 bytes / +75 tokens (publish changes from 594 bytes / 153 tokens to
  915 bytes / 228 tokens).
- Net: 210 - 75 = **135 tokens** and 784 - 321 = **463 bytes** saved, or
  **26.89243%** of the v1 turn tokens and **24.54931%** of its bytes.

The publish request itself remains 489 bytes / 125 tokens in both versions
because `expects_reply` was already part of the fixture. The remaining delta
is not an undispatched poll: it is the input content and delivery metadata
that must be transported to preserve the existing FIFO, delivery, and ack
semantics. No current implementation defect is indicated. If the owner keeps
42% as a hard threshold, it requires a separately approved protocol/design
optimization (for example, a more compact delivery envelope or a later
acknowledgement coalescing design), not a v1-baseline approval mutation.

| Portal operation | v1 total bytes/tokens | v2 total bytes/tokens |
|---|---:|---:|
| `portal_projection_attach` | 655 / 181 | 655 / 181 |
| `portal_projection_publish` | 594 / 153 | 915 / 228 |
| `portal_projection_get_pending_input` | 784 / 210 | removed |
| `portal_projection_acknowledge_input` | 508 / 139 | 508 / 139 |
| Full portal flow | 2,541 / 683 | 2,078 / 548 |

When an opted-in question finds no eligible input, the response omits
`pending_input`; the focused serialization test proves that its response has
zero added empty-field overhead. The legacy v1 calibration exercises that
empty response shape before injecting the fixture input for its explicit poll.

## Minimum owner approval packet

1. Decide whether the measured 26.89% turn-token reduction satisfies the
   intended outcome or whether the approximate 42% target remains a hard
   acceptance threshold.
2. If accepted, approve the exact tokenizer identity, fixture fingerprint,
   portal flow version/fingerprint, and every integer in the candidate JSON.
3. Record the owner decision reference and change only the candidate approval
   status through the integration/reconciliation lane; do not silently reuse
   v1 approval.
4. Decide whether a future v2 checker should be a separately named baseline
   or an explicitly version-selected path. The existing v1 checker must remain
   pointed at its approved v1 packet until that decision is implemented and
   reviewed.
5. If 42% remains a hard threshold, create and prioritize a follow-up for a
   separately reviewed compact-envelope or acknowledgement-coalescing design;
   this candidate must remain unapproved until that work is measured.

## Integration requirements

This candidate deliberately overlaps the portal/MCP/calibration surface with
other parallel work. Reconcile it in a dedicated integration lane before any
approval-state mutation. That lane must confirm the final combined wire schema,
rerun both calibration modes, preserve the approved v1 gate, and only then ask
for the owner decision above.
