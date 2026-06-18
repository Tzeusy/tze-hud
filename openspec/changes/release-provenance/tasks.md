# Tasks — Release Artifact Provenance

This change adds a release-integrity contract to the canonical Windows artifact. No
pipeline implementation begins until the change is reviewed and accepted; acceptance
authorizes the checksum-generation and smoke-test work.

## 1. Contract and review

- [ ] 1.1 Validate this change: `openspec validate release-provenance --strict`
- [ ] 1.2 Confirm the requirement adds no runtime behavior change (release/provenance only)
- [ ] 1.3 Confirm signing is explicitly optional/deferred for v1

## 2. Implementation

- [ ] 2.1 CI: produce `tze_hud.exe` (cross `x86_64-pc-windows-gnu`) and an auto-generated `tze_hud.exe.sha256` published as a workflow artifact
- [ ] 2.2 Update README §1.2 to point at the pipeline-generated checksum instead of the manual `sha256sum` instruction (README.md:225)
- [ ] 2.3 Document the verification procedure (compute + compare before activation) in the deployment docs / deployment automation

## 3. Verification (release-artifact smoke test)

- [x] 3.1 CI gate: build the real release `.exe`, compute its SHA-256, assert it matches the published checksum (provenance round-trip)
- [x] 3.2 Keep this gate distinct from the config-only `canonical-app-production-boot` gate
- [ ] 3.3 (Optional) headless smoke-boot of the packaged exe if feasible on the runner
