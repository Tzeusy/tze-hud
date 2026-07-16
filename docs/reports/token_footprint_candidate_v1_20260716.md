# Token-footprint candidate v1 — approval packet

Status: **candidate, unapproved**. This document is paste-ready evidence for an
owner decision. The implementation mandate did not approve these measured
values, and CI intentionally fails closed until approval is recorded.

## Decision requested

Approve or reject candidate v1 as the initial token-footprint comparison
authority for `hud-pngbn`. Approval applies to every integer below, the three
flow fingerprints, the fixture fingerprint, and the pinned tokenizer identity.
If approved, change only the candidate's `approval.status` from
`candidate_unapproved` to `owner_approved` and record the decision reference.

## Exact measured values

| Flow | Operation | Request bytes | Request tokens | Response bytes | Response tokens | Total bytes | Total tokens |
|---|---|---:|---:|---:|---:|---:|---:|
| `publish_to_zone` | `publish_to_zone` | 440 | 129 | 127 | 39 | 567 | 168 |
| `portal_projection` | `portal_projection_attach` | 512 | 142 | 120 | 33 | 632 | 175 |
| `portal_projection` | `portal_projection_publish` | 458 | 118 | 105 | 28 | 563 | 146 |
| `portal_projection` | `portal_projection_get_pending_input` | 334 | 99 | 416 | 103 | 750 | 202 |
| `portal_projection` | `portal_projection_acknowledge_input` | 385 | 105 | 89 | 25 | 474 | 130 |
| `publish_to_widget` | `publish_to_widget` | 320 | 100 | 117 | 36 | 437 | 136 |

| Flow total | Bytes | Tokens |
|---|---:|---:|
| `publish_to_zone` | 567 | 168 |
| `portal_projection` | 2419 | 653 |
| `publish_to_widget` | 437 | 136 |

## Compatibility identity

- Tokenizer: `tiktoken-rs` `0.12.0`, `o200k_base`.
- Vocabulary SHA-256: `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`.
- Fixture fingerprint: `blake3:2eb5b26b70f47913fc494dd716d93b62d1263aa82882372c4228687e2ed81f81`.
- Zone flow fingerprint: `blake3:9a4759392799d9def4aa0193a5b4e4b4675b28628800e10532420e3cd53ba490`.
- Portal flow fingerprint: `blake3:2bba2d14ffefce463c82b4abaad682ceacbabe48c3f7c5587814438066e8c946`.
- Widget flow fingerprint: `blake3:34d8d5dce48b7552b3c43e3bf3723d33bdb549960bfaaf12d884afdc425f9092`.

## Rationale and evidence

The fixture exercises the production MCP HTTP boundary against a real headless
runtime. The portal flow imports and uses the production `portal_client.py` and
routes through the in-process projection authority. Fixed content, IDs,
timestamps, ordering, and pending input make the payload repeatable. The live
owner token is used for authorization and replaced by `<OWNER_TOKEN>` only in
the measured bodies; credentials and HTTP headers are excluded.

Two complete runs emitted byte-identical JSON. Token counting is offline; there
are no model or external network calls. The candidate JSON is
`scripts/ci/token_footprint_candidate_v1.json`.

## Exact gate semantics

Every request, response, operation total, and flow total is compared for both
bytes and tokens using integer arithmetic. `measured * 100 > baseline * 105`
fails; increases from 1% through exactly 5% warn; decreases are improvements.
Tokenizer, vocabulary, fixture, flow, operation-set, missing-data, or approval
drift returns `baseline_incompatible` and fails closed.
