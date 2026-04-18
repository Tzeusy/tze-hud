## 1. Canonical Runtime Binary Target

- [ ] 1.1 Add a non-demo runtime application binary target as the canonical executable.
- [ ] 1.2 Implement startup configuration parsing for window mode/dimensions and endpoint settings.
- [ ] 1.3 Define and document deterministic Windows artifact name/output path for automation.
- [ ] 1.4 Add a binary discovery check (build metadata/test) proving canonical target is identifiable.

## 2. Windowed Runtime Network Services

- [ ] 2.1 Wire windowed runtime startup to initialize network runtime when endpoints are enabled.
- [ ] 2.2 Integrate MCP HTTP listener startup into canonical app lifecycle with configured bind address.
- [ ] 2.3 Enforce MCP authentication behavior for runtime-served HTTP calls.
- [ ] 2.4 Implement clean MCP listener teardown on runtime shutdown.
- [ ] 2.5 Add tests for enabled vs disabled endpoint startup behavior in windowed mode.

## 3. Cross-Machine Validation Flow

- [ ] 3.1 Update automation scripts to target canonical app artifact by default (Linux build -> Windows deploy).
- [ ] 3.2 Add MCP reachability gate before publish assertions in automation flow.
- [ ] 3.3 Add live publish smoke step (`publish_to_zone`) with structured success/failure output.
- [ ] 3.4 Add diagnostics for launch/runtime mismatches (artifact deployed but MCP endpoint unavailable).

## 4. Documentation and Operational Alignment

- [ ] 4.1 Update README and operator docs to distinguish canonical app binary from demo binaries.
- [ ] 4.2 Update user-test workflow docs/examples to require canonical app artifact and MCP reachability gate.
- [ ] 4.3 Document recommended Windows launch mode and endpoint configuration for remote publish testing.

## 5. Verification and Handoff

- [ ] 5.1 Verify local build of canonical runtime binary (including Windows cross-target artifact output).
- [ ] 5.2 Verify runtime exposes reachable MCP HTTP endpoint in windowed execution path.
- [ ] 5.3 Verify cross-machine publish smoke passes with authenticated request.
- [ ] 5.4 Record final runbook notes for future automation sessions.
