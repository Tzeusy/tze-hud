# Token-footprint candidate v1 — approval packet

Status: **owner-approved**. Owner decision `hud-ht1k7` approved this revised
candidate v1 packet on 2026-07-17, making it the initial token-footprint
comparison authority. The checker remains fail-closed for a missing approval,
decision reference, or any compatibility drift.

This revision supersedes the original packet values. Independent audit found
that the original zone/widget fixture measured the legacy bare-method dialect
instead of MCP-standard `tools/call`, omitted the production portal client's
`operation` discriminator fields, and lacked flow-version/decision-reference
compatibility gates. The earlier counts and fingerprints therefore do not
apply to this revised candidate.

## Approved decision

Owner decision `hud-ht1k7` approved candidate v1 as the initial
token-footprint comparison authority for `hud-pngbn`. Decision reference:
`hud-ht1k7`. Approval applies to every integer below, the three flow
fingerprints, the fixture fingerprint, and the pinned tokenizer identity. The
candidate records `approval.status = owner_approved` and
`approval.decision_reference = hud-ht1k7`; the checker requires both fields.

## Exact measured values

| Flow | Operation | Request bytes | Request tokens | Response bytes | Response tokens | Total bytes | Total tokens |
|---|---|---:|---:|---:|---:|---:|---:|
| `publish_to_zone` | `publish_to_zone` | 477 | 141 | 192 | 56 | 669 | 197 |
| `portal_projection` | `portal_projection_attach` | 535 | 148 | 120 | 33 | 655 | 181 |
| `portal_projection` | `portal_projection_publish` | 489 | 125 | 105 | 28 | 594 | 153 |
| `portal_projection` | `portal_projection_get_pending_input` | 368 | 107 | 416 | 103 | 784 | 210 |
| `portal_projection` | `portal_projection_acknowledge_input` | 419 | 114 | 89 | 25 | 508 | 139 |
| `publish_to_widget` | `publish_to_widget` | 357 | 112 | 182 | 56 | 539 | 168 |

| Flow total | Bytes | Tokens |
|---|---:|---:|
| `publish_to_zone` | 669 | 197 |
| `portal_projection` | 2541 | 683 |
| `publish_to_widget` | 539 | 168 |

## Compatibility identity

- Tokenizer: `tiktoken-rs` `0.12.0`, `o200k_base`.
- Vocabulary SHA-256: `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`.
- Canonical flow version: `1` for each flow.
- Fixture fingerprint: `blake3:86774ba0b39a5d1e812a9705fe0221d3071425d3b73a2ad07aada041530c1601`.
- Zone flow fingerprint: `blake3:82a47f35fb8516932e604a1148198cc7b1e5c4e35b33d5e366432cffba7de51e`.
- Portal flow fingerprint: `blake3:a0286e519a10f45b00ff6e578c6c81b95e9e2690d523293df89fc4c2c55273b3`.
- Widget flow fingerprint: `blake3:e37e93be69e0e0855bc099e25c970fdfdb2192baa22dcb8caa4af511b80232cd`.

## Rationale and evidence

The fixture exercises the production MCP HTTP boundary against a real headless
runtime. Zone and widget calls use the MCP-standard `tools/call` request and
response envelopes. The portal flow imports and uses the production
`portal_client.py`, includes the same operation discriminators as its CLI
commands, and routes through the in-process projection authority. Fixed
content, IDs, timestamps, ordering, and pending input make the payload
repeatable. The live owner token is used for authorization and replaced by
`<OWNER_TOKEN>` only in the measured bodies; credentials and HTTP headers are
excluded.

Two complete revised runs emitted byte-identical JSON with SHA-256
`6196d05575c8cb112a2ea7536ff2874e57d31d9da1ec408f46f2e6e590393927`.
Token counting is offline; there are no model or external network calls. The
candidate JSON is `scripts/ci/token_footprint_candidate_v1.json`.

## Exact gate semantics

Every request, response, operation total, and flow total is compared for both
bytes and tokens using integer arithmetic. `measured * 100 > baseline * 105`
fails; increases from 1% through exactly 5% warn; decreases are improvements.
Every changed value reports its absolute and percentage delta. Tokenizer,
vocabulary, fixture, flow version/fingerprint, operation set, count arithmetic,
missing data, approval status, or decision-reference drift returns
`baseline_incompatible` and fails closed.
