## 1. Protocol and Contract Alignment

- [ ] 1.1 Add `request_sequence` to `WidgetPublishResult` in the session proto, runtime handler, and protocol tests
- [ ] 1.2 Define and validate the publish-load benchmark artifact schema, benchmark identity fields, and byte-accounting labels against this OpenSpec change

## 2. Rust Harness Crate

- [ ] 2.1 Add a new workspace example crate for the publish-load harness with CLI argument parsing and benchmark mode selection
- [ ] 2.2 Implement target-registry resolution, auth/bootstrap handling, and single-session gRPC connection setup using `tze_hud_protocol`
- [ ] 2.3 Implement durable widget publish execution for paced and burst modes, including per-request RTT correlation via `request_sequence`

## 3. Metrics, Artifacts, and Validation Semantics

- [ ] 3.1 Emit canonical JSON artifacts plus any companion histogram/calibration files through the existing Layer 4 benchmark artifact path
- [ ] 3.2 Add raw metric reporting, `uncalibrated` status handling, informational-threshold reporting, and explicit byte-accounting mode fields
- [ ] 3.3 Support derived historical CSV summary emission compatible with `.claude/skills/user-test-performance/reference/results.csv`

## 4. Integration and Verification

- [ ] 4.1 Update `/user-test-performance` so its gRPC widget benchmark path invokes the Rust harness and preserves the existing target registry and comparison workflow
- [ ] 4.2 Add automated verification for request-correlation correctness, artifact generation, and CSV compatibility
- [ ] 4.3 Reconcile the implemented harness against doctrine, OpenSpec, and benchmark-history expectations before widening scope
