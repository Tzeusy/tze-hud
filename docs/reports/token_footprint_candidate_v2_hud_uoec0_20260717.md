# MCP response-diet token-footprint candidate (hud-uoec0)

**Status:** candidate-unapproved; not a CI comparison authority.

This packet records the deterministic, measured result of the `hud-uoec0`
MCP response-field diet and `tools/list` schema-budget ratchet. It does not
change pending-input delivery behavior or the `portal_projection_publish`
result shape, which remain owned by the separate `hud-vconx` work.

## Calibration

The canonical calibration was run twice before the change and twice after it.
Each pair was byte-identical. The headless runtime printed its usual
`XDG_RUNTIME_DIR` diagnostic on these successful offscreen runs.

| Measurement | Before v1 | Candidate v2 |
| --- | ---: | ---: |
| Fixture fingerprint | `blake3:86774ba0b39a5d1e812a9705fe0221d3071425d3b73a2ad07aada041530c1601` | `blake3:34452932b9fc5d4eea3889e4d88a836fa886adccaa5bfce63a0be6159c823980` |
| Portal flow version | 1 | 2 |
| Portal flow fingerprint | `blake3:a0286e519a10f45b00ff6e578c6c81b95e9e2690d523293df89fc4c2c55273b3` | `blake3:f1791479fd7731343e0a7a0a12474bfd4fd87119bfabb56add470f5b3bde4e27` |
| Portal total | 2,541 bytes / 683 tokens | 2,315 bytes / 641 tokens |

The portal total improves by 226 bytes (8.9%) and 42 tokens (6.1%).
`publish_to_zone` and `publish_to_widget` retain their v1 measurements and
fingerprints in this candidate.

| Portal operation | Response before | Response candidate | Total before | Total candidate |
| --- | ---: | ---: | ---: | ---: |
| `portal_projection_attach` | 120 B / 33 T | 81 B / 27 T | 655 B / 181 T | 616 B / 175 T |
| `portal_projection_publish` | 105 B / 28 T | 105 B / 28 T | 594 B / 153 T | 594 B / 153 T |
| `portal_projection_get_pending_input` | 416 B / 103 T | 267 B / 74 T | 784 B / 210 T | 635 B / 181 T |
| `portal_projection_acknowledge_input` | 89 B / 25 T | 51 B / 18 T | 508 B / 139 T | 470 B / 132 T |

## Tools/list budget ratchet

The schema measurements are deterministic and ratchet only downward:

| Surface | Before | Candidate ceiling | Change |
| --- | ---: | ---: | ---: |
| Portal tool definitions | 7,246 bytes | 5,742 bytes | -1,504 bytes (20.8%) |
| Full `tools/list` schema | 19,460 bytes | 17,956 bytes | -1,504 bytes (7.7%) |

The tool operation set remains eagerly discoverable because these are the
normative cooperative-projection operations. The reduced descriptions retain
field names, types, enum values, defaults, and validation bounds.

## Approval gate

The approved v1 baseline remains unchanged at
`scripts/ci/token_footprint_candidate_v1.json`. Checking this v2 measurement
against it correctly fails closed with:

```text
compatibility field changed: fixture_fingerprint
flow version changed: portal_projection
flow fingerprint changed: portal_projection
```

Checking the measurement against
`scripts/ci/token_footprint_candidate_v2_hud_uoec0.json` also fails closed:
that packet has `approval.status = candidate_unapproved` and no decision
reference. The generic checker was not weakened.

The smallest owner-approval packet is:

1. Approve or reject this exact v2 fixture and portal-flow fingerprint, all
   measured integers, and the portal flow-version increment to 2.
2. Supply the governing decision reference.
3. Decide whether this candidate replaces the v1 comparison authority and
   authorize the resulting CI/documents update.

Until then, this candidate must not be marked approved or substituted for the
v1 baseline.
