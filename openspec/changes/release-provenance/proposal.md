## Why

The `runtime-app-binary` capability already defines a stable Windows artifact identity
for automation (`openspec/specs/runtime-app-binary/spec.md` §Windows Artifact Identity
for Automation): the deployed binary is deterministically named `tze_hud.exe` so
deployment automation can find it. What it does **not** define is **integrity**: there
is no published checksum produced by the release pipeline, and the only integrity
guidance is a manual `sha256sum` note in the README (README.md §1.2). An operator
deploying `tze_hud.exe` over the network to a screen-owning host has no
pipeline-generated, verifiable provenance for the artifact they are about to activate.

For a runtime that owns the screen and is restarted unattended, a tampered or truncated
artifact must be detectable before activation. This change adds a Release Artifact
Provenance requirement: the release pipeline (not a manual step) produces a published
SHA-256 checksum alongside `tze_hud.exe`, and deployment automation verifies the
artifact against that checksum before activation. Cryptographic signing is named as an
explicit, optional, deferred follow-on so v1 does not over-commit to key
infrastructure.

## What Changes

- ADD a Release Artifact Provenance requirement to `runtime-app-binary`: every published
  `tze_hud.exe` is accompanied by a pipeline-generated SHA-256 checksum, and deployment
  automation verifies the artifact against the published checksum before activation.
- MARK signing (cosign/gpg) as optional and deferred within the requirement text.
- The README §1.2 manual `sha256sum` note becomes a pointer to the pipeline-generated
  checksum (implementation-time doc change, tracked in tasks).

This requirement is **verified** by the release-artifact smoke test (a CI gate that
builds the real `.exe`, computes its checksum, and asserts it matches the published
checksum), tracked as an implementation bead rather than a spec scenario.

## Impact

- Affected spec: `runtime-app-binary` (ADDED requirement).
- Affected CI: `.github/workflows/ci.yml` (checksum generation/publication; smoke test).
- Affected docs: `README.md` §1.2.
- No runtime behavior change; this is a release/provenance contract.
