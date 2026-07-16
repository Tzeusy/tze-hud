#!/usr/bin/env bash
# Run the low-power proxy benchmark with a verified two-logical-CPU limit.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "constrained-envelope lane requires Linux llvmpipe" >&2
  exit 2
fi
for command in cargo python3 taskset; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required command is unavailable: $command" >&2
    exit 2
  fi
done

repo_root="$(git rev-parse --show-toplevel)"
output_dir="${1:-$repo_root/test_results/constrained-envelope-budget}"
frames="${CONSTRAINED_ENVELOPE_FRAMES:-180}"
allowed_cpu_list="$(awk '/^Cpus_allowed_list:/ { print $2 }' /proc/self/status)"
cpu_pair="$(python3 "$repo_root/scripts/ci/select_constrained_cpu_pair.py" "$allowed_cpu_list")"

mkdir -p "$output_dir"
cargo build --release -p benchmark --features headless

# The benchmark records /proc/self/status and the selected wgpu adapter in its
# own artifact. The checker rejects the artifact if taskset did not reduce the
# actual affinity to exactly two CPUs or if llvmpipe was not selected.
benchmark_status=0
HEADLESS_FORCE_SOFTWARE=1 taskset --cpu-list "$cpu_pair" \
  "$repo_root/target/release/benchmark" \
  --constrained-envelope \
  --emit "$output_dir/benchmark.json" \
  --frames "$frames" || benchmark_status=$?

if [[ ! -s "$output_dir/benchmark.json" ]]; then
  echo "benchmark failed before emitting its constrained artifact (status $benchmark_status)" >&2
  if (( benchmark_status == 0 )); then
    benchmark_status=1
  fi
  exit "$benchmark_status"
fi

checker_status=0
python3 "$repo_root/scripts/ci/check_constrained_envelope.py" \
  --benchmark-json "$output_dir/benchmark.json" \
  --output-json "$output_dir/budget-gate.json" || checker_status=$?

# Preserve the benchmark's own definitive validation failure after allowing
# the constrained checker to emit its more diagnostic normalized artifact.
if (( benchmark_status != 0 )); then
  echo "benchmark reported a definitive validation failure (status $benchmark_status)" >&2
  exit "$benchmark_status"
fi
exit "$checker_status"
