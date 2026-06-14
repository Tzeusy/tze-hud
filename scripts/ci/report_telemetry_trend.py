#!/usr/bin/env python3
"""Surface cross-run numeric trend deltas for the Windows perf budget gate.

This is the *informational* trend surface that complements the hard PASS/FAIL
baseline gate in ``check_windows_perf_budgets.py`` (hud-ipmj0/PR #887). That gate
fails any regression against the checked-in baseline but does not retain prior
numeric values, so a within-budget drift (e.g. frame_time p99 14.1ms -> 14.8ms,
+5%) is invisible. validation.md §5.6 ("surface trends, not just pass/fail")
wants the numbers, not just the verdict.

Input is the budget-gate report JSON emitted by ``check_windows_perf_budgets.py
--output-json`` (schema ``tze_hud.windows_perf_budget_gate.v1``). Given a current
report and an optional previous report (restored from a cross-run cache/artifact),
this computes and prints per-metric deltas, and appends a Markdown table to the
GitHub Actions step summary when ``GITHUB_STEP_SUMMARY`` is set.

This script never fails the build on a within-budget delta: trend reporting is
informational. The first-run / no-prior-history case is handled gracefully
(prints a notice, exits 0).
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any


GATE_SCHEMA = "tze_hud.windows_perf_budget_gate.v1"

# Metrics surfaced in the trend table. Latency buckets are the high-value drift
# signal; the zero/baseline counters are surfaced too so a 0->N creep is visible
# in the trend table even when the hard gate is what actually fails the build.
TREND_METRICS = (
    "frame_time_p99",
    "frame_time_p99_9",
    "input_to_local_ack_p99",
    "input_to_scene_commit_p99",
    "input_to_next_present_p99",
    "scene_lock_misses",
    "invariant_violations",
)


def load_report(path: Path) -> dict[str, Any]:
    """Load and lightly validate a budget-gate report."""
    try:
        with path.open("r", encoding="utf-8") as handle:
            payload = json.load(handle)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"{path}: invalid JSON: {exc}") from exc
    except OSError as exc:
        raise SystemExit(f"{path}: unable to read: {exc}") from exc
    if not isinstance(payload, dict):
        raise SystemExit(f"{path}: expected object root")
    if payload.get("schema") != GATE_SCHEMA:
        raise SystemExit(
            f"{path}: unexpected schema {payload.get('schema')!r} "
            f"(expected {GATE_SCHEMA!r})"
        )
    return payload


def index_results(report: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
    """Index a report's results by ``(session, metric)``.

    Tolerates a missing/malformed ``results`` list (returns what it can) so a
    corrupt prior cache entry degrades to "no comparable history" rather than
    crashing the informational step.
    """
    results = report.get("results")
    if not isinstance(results, list):
        return {}
    index: dict[tuple[str, str], dict[str, Any]] = {}
    for result in results:
        if not isinstance(result, dict):
            continue
        session = result.get("session")
        metric = result.get("metric")
        if isinstance(session, str) and isinstance(metric, str):
            index[(session, metric)] = result
    return index


def observed_value(result: dict[str, Any]) -> int | float | None:
    """Extract the comparable numeric observation from a result entry.

    Latency results store ``observed_us``; counter results store ``observed``.
    Returns ``None`` for non-numeric/absent observations (skipped from deltas).
    """
    for key in ("observed_us", "observed"):
        if key in result:
            value = result[key]
            if isinstance(value, bool):
                return None
            if isinstance(value, (int, float)):
                return value
    return None


def unit_for(metric: str) -> str:
    return "us" if metric.endswith(("_p99", "_p99_9")) else ""


def format_value(value: int | float, unit: str) -> str:
    text = f"{value:g}"
    return f"{text}{unit}" if unit else text


def format_pct(prev: int | float, cur: int | float) -> str:
    """Format the percent change from ``prev`` to ``cur``.

    ``+0.0%`` when unchanged; ``n/a`` when the baseline is zero (a 0->N counter
    creep has no meaningful percentage — the absolute delta carries the signal).
    """
    if prev == cur:
        return "+0.0%"
    if prev == 0:
        return "n/a"
    pct = (cur - prev) / abs(prev) * 100.0
    sign = "+" if pct >= 0 else ""
    return f"{sign}{pct:.1f}%"


def compute_deltas(
    current: dict[str, Any],
    previous: dict[str, Any],
) -> list[dict[str, Any]]:
    """Compute per-(session, metric) deltas for the tracked trend metrics."""
    cur_index = index_results(current)
    prev_index = index_results(previous)
    deltas: list[dict[str, Any]] = []
    for (session, metric), cur_result in sorted(cur_index.items()):
        if metric not in TREND_METRICS:
            continue
        cur_value = observed_value(cur_result)
        if cur_value is None:
            continue
        prev_result = prev_index.get((session, metric))
        prev_value = observed_value(prev_result) if prev_result else None

        unit = unit_for(metric)
        within_budget = bool(cur_result.get("pass", True))
        entry: dict[str, Any] = {
            "session": session,
            "metric": metric,
            "current": cur_value,
            "previous": prev_value,
            "unit": unit,
            "within_budget": within_budget,
        }
        if prev_value is None:
            entry["delta"] = None
            entry["pct"] = None
        else:
            entry["delta"] = cur_value - prev_value
            entry["pct"] = format_pct(prev_value, cur_value)
        deltas.append(entry)
    return deltas


def render_line(entry: dict[str, Any]) -> str:
    """Render a single human-readable delta line for stdout/CI logs."""
    unit = entry["unit"]
    cur = format_value(entry["current"], unit)
    budget = "within budget" if entry["within_budget"] else "OVER BUDGET"
    if entry["previous"] is None:
        return f"{entry['session']}.{entry['metric']}: {cur} (no prior value, {budget})"
    prev = format_value(entry["previous"], unit)
    delta = entry["delta"]
    sign = "+" if delta >= 0 else ""
    delta_text = f"{sign}{format_value(delta, unit)}"
    return (
        f"{entry['session']}.{entry['metric']}: {prev} -> {cur} "
        f"({delta_text}, {entry['pct']}, {budget})"
    )


def render_markdown_table(deltas: list[dict[str, Any]]) -> str:
    """Render the deltas as a GitHub-flavored Markdown table."""
    lines = [
        "### Windows performance budget — cross-run trend",
        "",
        "Informational trend deltas vs the previous run's summary. The hard "
        "PASS/FAIL baseline gate is enforced separately and is unaffected by "
        "within-budget drift.",
        "",
        "| Session | Metric | Previous | Current | Delta | % | Budget |",
        "|---|---|---|---|---|---|---|",
    ]
    for entry in deltas:
        unit = entry["unit"]
        cur = format_value(entry["current"], unit)
        budget = "within" if entry["within_budget"] else "OVER"
        if entry["previous"] is None:
            prev = "—"
            delta_text = "—"
            pct = "—"
        else:
            prev = format_value(entry["previous"], unit)
            delta = entry["delta"]
            sign = "+" if delta >= 0 else ""
            delta_text = f"{sign}{format_value(delta, unit)}"
            pct = entry["pct"]
        lines.append(
            f"| {entry['session']} | {entry['metric']} | {prev} | {cur} | "
            f"{delta_text} | {pct} | {budget} |"
        )
    return "\n".join(lines) + "\n"


def write_step_summary(text: str) -> None:
    """Append ``text`` to the GitHub Actions step summary, if configured."""
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_path:
        return
    with open(summary_path, "a", encoding="utf-8") as handle:
        handle.write(text)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--current",
        type=Path,
        required=True,
        help="Path to the current run's budget-gate report JSON.",
    )
    parser.add_argument(
        "--previous",
        type=Path,
        help="Path to the previous run's budget-gate report JSON "
        "(restored from cross-run cache/artifact). Optional: when absent or "
        "missing on disk, the first-run case is handled gracefully.",
    )
    args = parser.parse_args()

    current = load_report(args.current)

    if args.previous is None or not args.previous.exists():
        notice = (
            "No prior Windows performance summary found — first run on this "
            "cache key. Trend deltas will appear on the next run."
        )
        print(notice)
        write_step_summary(
            "### Windows performance budget — cross-run trend\n\n"
            f"{notice}\n"
        )
        return 0

    previous = load_report(args.previous)
    deltas = compute_deltas(current, previous)

    if not deltas:
        notice = (
            "No comparable trend metrics between the current and previous "
            "Windows performance summaries."
        )
        print(notice)
        write_step_summary(
            "### Windows performance budget — cross-run trend\n\n"
            f"{notice}\n"
        )
        return 0

    print("Windows performance budget — cross-run trend deltas:")
    for entry in deltas:
        print(f"  {render_line(entry)}")

    write_step_summary(render_markdown_table(deltas))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
