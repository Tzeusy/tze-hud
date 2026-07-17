# Combined MCP token-footprint candidate v2 (hud-veklx)

**Status:** `candidate_unapproved`; `decision_reference: null`. This is an
owner-approval packet, not a comparison authority. The approved v1 packet at
`scripts/ci/token_footprint_candidate_v1.json` remains the only CI baseline.

## What is combined

This candidate composes two preserved inputs without changing their source
branches or approval state:

- compact MCP response serialization: request-scoped `projection_id`, default
  `delivered` state, default `private` classification, and fixed success
  summaries are omitted;
- `expects_reply = true` publishes the output first, then uses the exact normal
  `get_pending_input` authority path with `max_items = 1`.

The latter keeps FIFO eligibility, pending/deferred-to-delivered transitions,
delivery timestamps, repaint scheduling, operation audit records, terminal
acknowledgement idempotency, and logical-output replay behavior unchanged. An
empty piggyback is omitted; omitted or false `expects_reply` never reads or
mutates the inbox.

## Repeatability and identity

Two fresh headless MCP/runtime calibration runs in
`combined-candidate-v2` mode were byte-identical:

- SHA-256 of each output: `100cd1aa168fdcb34d8e990d0832cddc45e0d92707fd625a467eb46802c21a46`
- Fixture fingerprint: `blake3:70f65a5eb9beea36e6b9e0de4f62de2de9ec88a60ce28c3062df11c6f5d7c26e`
- Tokenizer: `o200k_base`, `tiktoken-rs` `0.12.0`, vocabulary `sha256:446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`
- Counting policy: independent canonical JSON-RPC bodies, UTF-8 bytes,
  `encode_with_special_tokens`, and integer operation/flow sums.

Canonical flow version: `2` for `portal_projection`; its fingerprint is
`blake3:891009a81b03f7196b180ffa758ed0007256df290a339d8ce7b4436d9d82359d`.
Canonical flow version: `1` for `publish_to_widget`; its fingerprint is
`blake3:e37e93be69e0e0855bc099e25c970fdfdb2192baa22dcb8caa4af511b80232cd`.
Canonical flow version: `1` for `publish_to_zone`; its fingerprint is
`blake3:82a47f35fb8516932e604a1148198cc7b1e5c4e35b33d5e366432cffba7de51e`.

## Measured integers

| Flow | Operation | Request bytes | Request tokens | Response bytes | Response tokens | Total bytes | Total tokens |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `portal_projection` | `portal_projection_acknowledge_input` | 419 | 114 | 51 | 18 | 470 | 132 |
| `portal_projection` | `portal_projection_attach` | 535 | 148 | 81 | 27 | 616 | 175 |
| `portal_projection` | `portal_projection_publish` | 489 | 125 | 265 | 73 | 754 | 198 |
| `publish_to_widget` | `publish_to_widget` | 357 | 112 | 182 | 56 | 539 | 168 |
| `publish_to_zone` | `publish_to_zone` | 477 | 141 | 192 | 56 | 669 | 197 |

| Flow | Total bytes | Total tokens |
| --- | ---: | ---: |
| `portal_projection` | 1840 | 505 |
| `publish_to_widget` | 539 | 168 |
| `publish_to_zone` | 669 | 197 |

## Turn economics: gross removal versus net saving

The canonical conversational turn excludes one-time attach. Approved v1 uses
`publish + get_pending_input + acknowledge_input` (`1,886 bytes / 502 tokens`);
the combined candidate uses `publish(+pending_input) + acknowledge_input`
(`1,224 bytes / 330 tokens`).

- Gross avoided poll: `784 bytes / 210 tokens`.
- Required piggyback payload raises publish by `160 bytes / 45 tokens` versus
  the approved v1 publish shape.
- The response diet also saves `38 bytes / 7 tokens` on acknowledgement.
- Net canonical-turn saving: `662 bytes / 172 tokens` (35.10% bytes, 34.26%
  tokens).

Including attach, the measured portal flow is `2,541 bytes / 683 tokens` in
approved v1 and `1,840 bytes / 505 tokens` here: `701 bytes / 178 tokens`
lower (27.59% bytes, 26.06% tokens). The headline is deliberately the net
turn saving, not the larger gross poll removal.

## Schema-budget ratchet

The `tools/list` assertion remains a one-way ratchet: portal projection schema
bytes must stay at or below `5,742`, and complete `tools/list` schema bytes at
or below `17,956`. The candidate neither widens those ceilings nor makes
rarely used operations undiscoverable.

## Fail-closed approval boundary

The generic checker is unchanged. Comparing this packet to itself fails because
its `approval.status` is `candidate_unapproved`. Comparing the combined
measurement to the approved v1 baseline fails on `fixture_fingerprint`, portal
flow version, portal flow fingerprint, and the removed explicit-poll operation
set. The `legacy-v1` calibration still executes the explicit-poll call shape
and exits successfully, but its dieted wire representation correctly fails the
v1 baseline on fixture and portal-flow fingerprints. No checker, CI baseline,
or approval field is weakened or updated by this change.

The owner must approve or reject this exact packet: every integer above, all
three flow fingerprints and versions, fixture/tokenizer identity, schema
ceilings, and the explicit gross-versus-net economics. An approval, if granted,
must supply a decision reference in a separate authority-changing change.

## Validation scope

The two measurements were generated serially with no concurrent
`token_footprint_calibration` process observed. A second two-run sweep used the
CI setting `HEADLESS_FORCE_SOFTWARE=1`; both runs exited `0`, were byte-identical
to each other and to the first pair, and produced the same SHA-256 above.

Each software-forced run printed five `XDG_RUNTIME_DIR not set in the
environment.` messages from the headless graphics stack. They did not change
the exit status or output and are recorded as a headless-environment diagnostic,
not a GPU contention failure. The process check only established that no other
token-footprint calibration was running: an unrelated compositor test executable
from another worktree was already present during later GPU-unit validation, so
host-GPU isolation cannot be claimed. The byte-identical serial calibration is
deterministic evidence for this fixture, not a physical-GPU-isolation claim.
Broader GPU-sensitive compositor suites are not claimed by this packet; they
remain separately scoped in PR validation.
