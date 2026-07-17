# Token-footprint calibration

This Layer-3 calibration measures the exact canonical JSON-RPC request and
response bodies for three LLM-facing flows against a real headless runtime and
its MCP HTTP server:

1. one `publish_to_zone` notification;
2. portal `attach`, append-only `publish`, bounded `get_pending_input`, and
   `acknowledge_input`;
3. one single-parameter `publish_to_widget`.

The Python flow driver imports and uses the production
`.claude/skills/hud-projection/scripts/portal_client.py` transport. Zone and
widget publishes use the MCP-standard `tools/call` envelope; the portal turn
explicitly uses the client's raw bare-method compatibility transport rather
than its policy-selected default dialect, and includes the same `operation`
discriminator fields as its CLI commands. The fixture supplies fixed IDs,
timestamps, content, order, and response input. The live owner token is
required for the real calls but is replaced with the canonical
`<OWNER_TOKEN>` sentinel in measured bodies. HTTP headers and credentials are
excluded. No model or external network call occurs.

`tiktoken-rs` 0.12.0 counts each UTF-8 body independently with the bundled
`o200k_base` vocabulary. Operation and flow totals are integer sums. The
tokenizer, vocabulary, flow version, fixture, and flow fingerprints make
incompatible baselines fail closed.

CI runs the calibration twice and requires byte-identical JSON. It then compares
every request, response, operation total, and flow total in both bytes and
tokens. The exact rule is:

```text
measured * 100 > baseline * 105  => fail
baseline < measured <= 105%      => warning
measured < baseline              => improvement
```

Run locally:

```bash
mkdir -p test_results/token-footprint
HEADLESS_FORCE_SOFTWARE=1 cargo run -p benchmark --features headless \
  --bin token_footprint_calibration -- \
  --output test_results/token-footprint/measurement.json
python3 scripts/ci/check_token_footprint.py \
  --measurement test_results/token-footprint/measurement.json \
  --baseline scripts/ci/token_footprint_candidate_v1.json \
  --output test_results/token-footprint/gate-report.json
```

The checked-in v1 values are an explicitly unapproved candidate, so the gate
fails closed with `baseline_incompatible`. Promotion to comparison authority
requires explicit owner approval of every value in the candidate plus a
non-empty decision reference. Baseline changes then require explicit owner
review. Changing a tokenizer identity, fixture fingerprint, flow version,
flow fingerprint, operation set, approval state, decision reference, or count
arithmetic is not a performance comparison; the gate reports
`baseline_incompatible`.
