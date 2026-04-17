## 1. Contract Reconciliation

- [x] 1.1 Reconcile RFC 0011 upload-start acknowledgement and correlation semantics against the main session-protocol and resource-store specs
- [x] 1.2 Reconcile RFC 0005 combined-envelope sections against the split-envelope resident session model used by the main v1 spec
- [x] 1.3 Reconcile scene-resource budget charging semantics to reference time instead of upload-storage time
- [x] 1.4 Decide and document the v1 boundary for agent-uploaded fonts versus runtime-owned zone/component typography
- [x] 1.5 Sync the temporary `resident-scene-resource-upload` synthesis slice into its authoritative homes and mark its archive posture
- [x] 1.6 Sync the resident scene-resource upload delta into the authoritative session-protocol and resource-store specs after signoff

## 2. Schema

- [ ] 2.1 Add resident scene-resource upload messages and response fields to `crates/tze_hud_protocol/proto/session.proto`
- [ ] 2.2 Regenerate protocol bindings and update compile surfaces that consume the session schema

## 3. Runtime Implementation

- [ ] 3.1 Implement `ResourceUploadStart` handling in the resident session server with capability, dedup, and inline fast-path checks
- [ ] 3.2 Implement chunked upload state, `upload_id` acknowledgement, and `ResourceUploadComplete` handling
- [ ] 3.3 Emit `ResourceStored` and `ResourceErrorResponse` according to the reconciled contract
- [ ] 3.4 Preserve transactional delivery and backpressure semantics for the resident upload message family
- [ ] 3.5 Apply upload rate limiting at the transport/session layer while keeping scene-resource budget charging at reference time

## 4. Verification

- [ ] 4.1 Add protocol conformance tests for resident upload envelope allocation and response correlation
- [ ] 4.2 Add runtime/resource-store tests for inline, chunked, dedup, capability denial, and concurrent-upload rejection paths
- [ ] 4.3 Add protocol/runtime tests for upload traffic-class assignment, rate limiting, and transport backpressure behavior
- [ ] 4.4 Add at least one resident integration test that uploads a resource and references it from a scene node

## 5. Consumer Repair

- [ ] 5.1 Update resident helper surfaces to upload a small PNG and return `ResourceId`
- [ ] 5.2 Convert resident exemplar/user-test flows that currently rely on placeholder upload behavior to the real session-stream upload path
