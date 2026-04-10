---
name: user-test-performance
description: Use when running publish-load performance checks. gRPC widget publish benchmarks run through the Rust `widget_publish_load_harness`; MCP benchmarking workflow remains unchanged.
---

# User Test Performance

This skill runs widget publish performance checks while preserving the historical
comparison ledger workflow.

## Scope

- gRPC widget publish benchmarking: **Rust harness path** via
  `examples/widget_publish_load_harness`.
- Historical comparison: append/compare through
  `test_results/benchmark_history/results.csv`.
- MCP benchmarking: unchanged in this tranche.

## gRPC Benchmark Command

```bash
python3 .claude/skills/user-test-performance/scripts/grpc_widget_publish_perf.py \
  --target-id user-test-windows-tailnet \
  --mode burst \
  --publish-count 1000
```

Common options:

- `--targets-file` (default: `targets/publish_load_targets.toml`)
- `--mode burst|paced`
- `--publish-count`, `--duration-s`, `--target-rate-rps`
- `--widget-name`, `--instance-id`, `--payload-profile`
- `--output-json` (artifact path)
- `--layer4-output-root` (default: `test_results/`, writes Layer 4 manifest run)
- `--results-csv` (default: `test_results/benchmark_history/results.csv`)
- `--cargo-profile release|debug`

The wrapper executes:

```bash
cargo run -p widget_publish_load_harness --release -- ...
```

Then it appends a stable summary row derived from the JSON artifact into the
historical CSV, emits a Layer 4 artifact run (`manifest.json` benchmark entry
includes `publish_load.json`), and prints deltas vs the latest prior row with the same
`benchmark_key`.
