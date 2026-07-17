#!/usr/bin/env python3
"""Contract tests for the token-footprint baseline gate."""

import copy
import importlib.util
import json
import pathlib
import re
import unittest


SCRIPT = pathlib.Path(__file__).with_name("check_token_footprint.py")
SPEC = importlib.util.spec_from_file_location("check_token_footprint", SCRIPT)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def fixture(value=100):
    metric = {
        "request": {"bytes": value, "tokens": value},
        "response": {"bytes": value, "tokens": value},
        "total": {"bytes": value * 2, "tokens": value * 2},
    }
    return {
        "schema_version": 1,
        "tokenizer": {
            "name": "o200k_base",
            "implementation": "tiktoken-rs",
            "version": "0.12.0",
            "vocab_fingerprint": "sha256:fixture",
        },
        "fixture_fingerprint": "sha256:fixture",
        "flows": {
            "publish_to_zone": {
                "flow_version": 1,
                "flow_fingerprint": "sha256:zone",
                "operations": {"publish_to_zone": copy.deepcopy(metric)},
                "total": copy.deepcopy(metric["total"]),
            },
            "portal_projection": {
                "flow_version": 1,
                "flow_fingerprint": "sha256:portal",
                "operations": {"attach": copy.deepcopy(metric)},
                "total": copy.deepcopy(metric["total"]),
            },
            "publish_to_widget": {
                "flow_version": 1,
                "flow_fingerprint": "sha256:widget",
                "operations": {"publish_to_widget": copy.deepcopy(metric)},
                "total": copy.deepcopy(metric["total"]),
            },
        },
    }


def approve(document):
    document["approval"] = {
        "status": "owner_approved",
        "decision_reference": "hud-test-decision",
    }


class GateTests(unittest.TestCase):
    def test_exact_five_percent_is_warning_but_six_percent_fails(self):
        baseline = fixture()
        approve(baseline)
        at_limit = fixture(105)
        report = checker.compare(at_limit, baseline)
        self.assertEqual(report["status"], "warning")
        self.assertFalse(report["regressions"])
        self.assertEqual(report["warnings"][0]["absolute_delta"], 5)
        self.assertEqual(report["warnings"][0]["percentage_delta"], 5.0)

        over_limit = fixture(106)
        report = checker.compare(over_limit, baseline)
        self.assertEqual(report["status"], "failed")
        self.assertTrue(report["regressions"])
        self.assertEqual(report["regressions"][0]["absolute_delta"], 6)
        self.assertEqual(report["regressions"][0]["percentage_delta"], 6.0)

    def test_compares_every_operation_direction_and_flow_total(self):
        baseline = fixture()
        approve(baseline)
        measurement = fixture()
        measurement["flows"]["portal_projection"]["operations"]["attach"]["response"][
            "tokens"
        ] = 106
        measurement["flows"]["portal_projection"]["operations"]["attach"]["total"][
            "tokens"
        ] = 206
        measurement["flows"]["portal_projection"]["total"]["tokens"] = 206
        report = checker.compare(measurement, baseline)
        regression_paths = {entry["path"] for entry in report["regressions"]}
        warning_paths = {entry["path"] for entry in report["warnings"]}
        self.assertEqual(
            regression_paths,
            {"portal_projection.operations.attach.response.tokens"},
        )
        self.assertEqual(
            warning_paths,
            {
                "portal_projection.operations.attach.total.tokens",
                "portal_projection.total.tokens",
            },
        )

    def test_fingerprint_drift_is_incompatible_not_a_regression(self):
        baseline = fixture()
        approve(baseline)
        measurement = fixture()
        measurement["flows"]["publish_to_zone"]["flow_fingerprint"] = "sha256:changed"
        report = checker.compare(measurement, baseline)
        self.assertEqual(report["status"], "baseline_incompatible")
        self.assertFalse(report["regressions"])

    def test_unapproved_baseline_fails_closed(self):
        report = checker.compare(fixture(), fixture())
        self.assertEqual(report["status"], "baseline_incompatible")
        self.assertIn("owner-approved", " ".join(report["incompatibilities"]))

    def test_missing_metric_fails_closed_as_incompatible(self):
        baseline = fixture()
        approve(baseline)
        measurement = fixture()
        del measurement["flows"]["publish_to_widget"]["total"]["tokens"]
        report = checker.compare(measurement, baseline)
        self.assertEqual(report["status"], "baseline_incompatible")
        self.assertIn("missing or invalid integer metric", " ".join(report["incompatibilities"]))

    def test_approved_baseline_requires_decision_reference(self):
        baseline = fixture()
        baseline["approval"] = {"status": "owner_approved"}
        report = checker.compare(fixture(), baseline)
        self.assertEqual(report["status"], "baseline_incompatible")
        self.assertIn("decision reference", " ".join(report["incompatibilities"]))

    def test_flow_version_drift_is_incompatible(self):
        baseline = fixture()
        approve(baseline)
        measurement = fixture()
        measurement["flows"]["portal_projection"]["flow_version"] = 2
        report = checker.compare(measurement, baseline)
        self.assertEqual(report["status"], "baseline_incompatible")
        self.assertIn("flow version changed", " ".join(report["incompatibilities"]))

    def test_inconsistent_operation_and_flow_totals_fail_closed(self):
        baseline = fixture()
        approve(baseline)
        measurement = fixture()
        measurement["flows"]["publish_to_zone"]["operations"]["publish_to_zone"][
            "total"
        ]["tokens"] += 1
        measurement["flows"]["publish_to_widget"]["total"]["bytes"] += 1
        report = checker.compare(measurement, baseline)
        self.assertEqual(report["status"], "baseline_incompatible")
        reasons = " ".join(report["incompatibilities"])
        self.assertIn("operation total mismatch", reasons)
        self.assertIn("flow total mismatch", reasons)


class CandidatePacketTests(unittest.TestCase):
    def setUp(self):
        self.root = SCRIPT.parents[2]
        self.candidate = json.loads(
            (self.root / "scripts/ci/token_footprint_candidate_v1.json").read_text(
                encoding="utf-8"
            )
        )
        self.packet = (
            self.root / "docs/reports/token_footprint_candidate_v1_20260716.md"
        ).read_text(encoding="utf-8")

    def test_candidate_records_revised_owner_approval(self):
        self.assertEqual(self.candidate["approval"]["status"], "owner_approved")
        self.assertEqual(
            self.candidate["approval"]["decision_reference"],
            "hud-ht1k7",
        )
        self.assertRegex(
            self.packet,
            r"Decision reference:\s+`hud-ht1k7`\.",
        )

    def test_approved_candidate_is_accepted_by_fail_closed_gate(self):
        report = checker.compare(copy.deepcopy(self.candidate), self.candidate)
        self.assertEqual(report["status"], "passed")
        self.assertFalse(report["incompatibilities"])

    def test_markdown_operation_table_matches_candidate_json(self):
        rows = re.findall(
            r"^\| `([^`]+)` \| `([^`]+)` \| (\d+) \| (\d+) \| (\d+) \| "
            r"(\d+) \| (\d+) \| (\d+) \|$",
            self.packet,
            flags=re.MULTILINE,
        )
        self.assertTrue(rows, "candidate packet operation table is missing")
        expected = []
        for flow_name, flow in self.candidate["flows"].items():
            for operation_name, operation in flow["operations"].items():
                expected.append(
                    (
                        flow_name,
                        operation_name,
                        str(operation["request"]["bytes"]),
                        str(operation["request"]["tokens"]),
                        str(operation["response"]["bytes"]),
                        str(operation["response"]["tokens"]),
                        str(operation["total"]["bytes"]),
                        str(operation["total"]["tokens"]),
                    )
                )
        self.assertEqual(sorted(rows), sorted(expected))

    def test_markdown_flow_totals_and_identity_match_candidate_json(self):
        rows = re.findall(
            r"^\| `([^`]+)` \| (\d+) \| (\d+) \|$",
            self.packet,
            flags=re.MULTILINE,
        )
        expected = sorted(
            (
                flow_name,
                str(flow["total"]["bytes"]),
                str(flow["total"]["tokens"]),
            )
            for flow_name, flow in self.candidate["flows"].items()
        )
        self.assertEqual(sorted(rows), expected)
        tokenizer = self.candidate["tokenizer"]
        for identity in (
            tokenizer["name"],
            tokenizer["implementation"],
            tokenizer["version"],
            tokenizer["vocab_fingerprint"].removeprefix("sha256:"),
            self.candidate["fixture_fingerprint"],
        ):
            self.assertIn(identity, self.packet)
        for flow in self.candidate["flows"].values():
            self.assertIn(f"Canonical flow version: `{flow['flow_version']}`", self.packet)
            self.assertIn(flow["flow_fingerprint"], self.packet)


if __name__ == "__main__":
    unittest.main()
