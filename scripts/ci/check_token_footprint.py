#!/usr/bin/env python3
"""Fail-closed token-footprint regression gate for deterministic MCP flows."""

import argparse
import json
import pathlib
import sys


METRIC_NAMES = ("bytes", "tokens")
SIDES = ("request", "response", "total")


def _incompatibility_checks(measurement, baseline):
    reasons = []
    approval = baseline.get("approval", {})
    if approval.get("status") != "owner_approved":
        reasons.append("baseline is not owner-approved")
    elif not isinstance(approval.get("decision_reference"), str) or not approval[
        "decision_reference"
    ].strip():
        reasons.append("owner-approved baseline is missing a decision reference")
    for field in ("schema_version", "tokenizer", "fixture_fingerprint"):
        if field not in measurement or field not in baseline:
            reasons.append(f"missing compatibility field: {field}")
        elif measurement[field] != baseline[field]:
            reasons.append(f"compatibility field changed: {field}")
    measured_flows = measurement.get("flows")
    baseline_flows = baseline.get("flows")
    if not isinstance(measured_flows, dict) or not isinstance(baseline_flows, dict):
        reasons.append("missing flows object")
        return reasons
    if set(measured_flows) != set(baseline_flows):
        reasons.append("flow set changed")
        return reasons
    for flow_name in sorted(baseline_flows):
        measured = measured_flows[flow_name]
        expected = baseline_flows[flow_name]
        if not isinstance(measured, dict) or not isinstance(expected, dict):
            reasons.append(f"invalid flow object: {flow_name}")
            continue
        measured_flow_version = measured.get("flow_version")
        expected_flow_version = expected.get("flow_version")
        if not isinstance(measured_flow_version, int) or isinstance(
            measured_flow_version, bool
        ):
            reasons.append(f"missing or invalid flow version: measurement:{flow_name}")
        if not isinstance(expected_flow_version, int) or isinstance(
            expected_flow_version, bool
        ):
            reasons.append(f"missing or invalid flow version: baseline:{flow_name}")
        if measured_flow_version != expected_flow_version:
            reasons.append(f"flow version changed: {flow_name}")
        measured_fingerprint = measured.get("flow_fingerprint")
        expected_fingerprint = expected.get("flow_fingerprint")
        fingerprints_valid = True
        for label, fingerprint in (
            ("measurement", measured_fingerprint),
            ("baseline", expected_fingerprint),
        ):
            if not isinstance(fingerprint, str) or not fingerprint.strip():
                reasons.append(
                    f"missing or invalid flow fingerprint: {label}:{flow_name}"
                )
                fingerprints_valid = False
        if fingerprints_valid and measured_fingerprint != expected_fingerprint:
            reasons.append(f"flow fingerprint changed: {flow_name}")
        measured_ops = measured.get("operations")
        expected_ops = expected.get("operations")
        if not isinstance(measured_ops, dict) or not isinstance(expected_ops, dict):
            reasons.append(f"missing operations object: {flow_name}")
        elif set(measured_ops) != set(expected_ops):
            reasons.append(f"operation set changed: {flow_name}")
        else:
            for operation_name in sorted(expected_ops):
                for label, operation in (
                    ("measurement", measured_ops[operation_name]),
                    ("baseline", expected_ops[operation_name]),
                ):
                    reasons.extend(
                        _validate_operation(
                            operation,
                            f"{label}:{flow_name}.{operation_name}",
                        )
                    )
                    reasons.extend(
                        _validate_operation_total(
                            operation,
                            f"{label}:{flow_name}.{operation_name}",
                        )
                    )
        for label, flow in (("measurement", measured), ("baseline", expected)):
            reasons.extend(_validate_counts(flow.get("total"), f"{label}:{flow_name}.total"))
            reasons.extend(_validate_flow_total(flow, f"{label}:{flow_name}"))
    return reasons


def _validate_operation(operation, path):
    if not isinstance(operation, dict):
        return [f"invalid operation object: {path}"]
    reasons = []
    for side in SIDES:
        reasons.extend(_validate_counts(operation.get(side), f"{path}.{side}"))
    return reasons


def _validate_counts(counts, path):
    if not isinstance(counts, dict):
        return [f"missing counts object: {path}"]
    reasons = []
    for metric in METRIC_NAMES:
        value = counts.get(metric)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            reasons.append(f"missing or invalid integer metric: {path}.{metric}")
    return reasons


def _metric(counts, metric):
    if not isinstance(counts, dict):
        return None
    value = counts.get(metric)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        return None
    return value


def _validate_operation_total(operation, path):
    if not isinstance(operation, dict):
        return []
    reasons = []
    for metric in METRIC_NAMES:
        request = _metric(operation.get("request"), metric)
        response = _metric(operation.get("response"), metric)
        total = _metric(operation.get("total"), metric)
        if None not in (request, response, total) and total != request + response:
            reasons.append(f"operation total mismatch: {path}.{metric}")
    return reasons


def _validate_flow_total(flow, path):
    if not isinstance(flow, dict) or not isinstance(flow.get("operations"), dict):
        return []
    reasons = []
    for metric in METRIC_NAMES:
        total = _metric(flow.get("total"), metric)
        operation_totals = [
            _metric(operation.get("total"), metric)
            for operation in flow["operations"].values()
            if isinstance(operation, dict)
        ]
        if (
            total is not None
            and len(operation_totals) == len(flow["operations"])
            and all(value is not None for value in operation_totals)
            and total != sum(operation_totals)
        ):
            reasons.append(f"flow total mismatch: {path}.{metric}")
    return reasons


def _metric_values(document):
    for flow_name, flow in sorted(document["flows"].items()):
        for operation_name, operation in sorted(flow["operations"].items()):
            for side in SIDES:
                for metric in METRIC_NAMES:
                    path = f"{flow_name}.operations.{operation_name}.{side}.{metric}"
                    yield path, operation[side][metric]
        for metric in METRIC_NAMES:
            yield f"{flow_name}.total.{metric}", flow["total"][metric]


def compare(measurement, baseline):
    """Compare every integer metric using exact 5% arithmetic."""
    incompatibilities = _incompatibility_checks(measurement, baseline)
    if incompatibilities:
        return {
            "schema_version": 1,
            "status": "baseline_incompatible",
            "threshold_percent": 5,
            "incompatibilities": incompatibilities,
            "regressions": [],
            "warnings": [],
            "improvements": [],
        }

    measured = dict(_metric_values(measurement))
    expected = dict(_metric_values(baseline))
    if set(measured) != set(expected):
        return {
            "schema_version": 1,
            "status": "baseline_incompatible",
            "threshold_percent": 5,
            "incompatibilities": ["metric set changed"],
            "regressions": [],
            "warnings": [],
            "improvements": [],
        }

    regressions = []
    warnings = []
    improvements = []
    for path in sorted(expected):
        baseline_value = expected[path]
        measured_value = measured[path]
        absolute_delta = measured_value - baseline_value
        percentage_delta = (
            None
            if baseline_value == 0
            else round(absolute_delta * 100 / baseline_value, 6)
        )
        entry = {
            "path": path,
            "baseline": baseline_value,
            "measured": measured_value,
            "absolute_delta": absolute_delta,
            "percentage_delta": percentage_delta,
        }
        if measured_value * 100 > baseline_value * 105:
            regressions.append(entry)
        elif measured_value > baseline_value:
            warnings.append(entry)
        elif measured_value < baseline_value:
            improvements.append(entry)

    status = "failed" if regressions else "warning" if warnings else "passed"
    return {
        "schema_version": 1,
        "status": status,
        "threshold_percent": 5,
        "incompatibilities": [],
        "regressions": regressions,
        "warnings": warnings,
        "improvements": improvements,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--measurement", required=True, type=pathlib.Path)
    parser.add_argument("--baseline", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()

    measurement = json.loads(args.measurement.read_text(encoding="utf-8"))
    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    report = compare(measurement, baseline)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["status"] in {"passed", "warning"} else 1


if __name__ == "__main__":
    sys.exit(main())
