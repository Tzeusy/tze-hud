## Why

The project currently lacks a canonical, non-demo runtime executable that can be built for Windows and used for real cross-machine operations. This prevents reliable MCP HTTP publishing workflows and causes automation to target `vertical_slice`, which is a demo path and not an application runtime surface.

## What Changes

- Add a canonical runtime application binary target for `tze_hud` (separate from demo/example binaries).
- Define runtime requirements for starting windowed rendering with network services enabled through configuration.
- Define runtime requirements for MCP HTTP listener lifecycle (bind, auth enforcement, and shutdown behavior).
- Define deployability requirements for Linux cross-build to Windows artifact, remote launch, and live MCP zone publish validation.
- Update operator/developer docs so automation targets the canonical app artifact, not `vertical_slice`.

## Capabilities

### New Capabilities
- `runtime-app-binary`: Canonical application executable requirements, including startup configuration, executable identity, and non-demo positioning.
- `windowed-runtime-network-services`: Requirements for enabling network services (including MCP HTTP) in the windowed runtime path.
- `cross-machine-runtime-validation`: Requirements for validating Linux-build -> Windows-deploy -> runtime launch -> live MCP publish.

### Modified Capabilities
- None.

## Impact

- Affected code: runtime entrypoint/binary target wiring, windowed runtime startup path, MCP HTTP server lifecycle integration, deployment automation scripts, and operator docs.
- Affected APIs: MCP HTTP endpoint availability and authentication behavior from the canonical app binary.
- Affected systems: Linux build pipeline, Windows deployment/test flow over SSH/SCP, and user-test automation.
