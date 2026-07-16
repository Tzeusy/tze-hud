#!/usr/bin/env python3
"""Contract tests for the token-footprint baseline gate."""

import copy
import importlib.util
import pathlib
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
                "flow_fingerprint": "sha256:zone",
                "operations": {"publish_to_zone": copy.deepcopy(metric)},
                "total": copy.deepcopy(metric["total"]),
            },
            "portal_projection": {
                "flow_fingerprint": "sha256:portal",
                "operations": {"attach": copy.deepcopy(metric)},
                "total": copy.deepcopy(metric["total"]),
            },
            "publish_to_widget": {
                "flow_fingerprint": "sha256:widget",
                "operations": {"publish_to_widget": copy.deepcopy(metric)},
                "total": copy.deepcopy(metric["total"]),
            },
        },
    }


class GateTests(unittest.TestCase):
    def test_exact_five_percent_is_warning_but_six_percent_fails(self):
        baseline = fixture()
        baseline["approval"] = {"status": "owner_approved"}
        at_limit = fixture(105)
        report = checker.compare(at_limit, baseline)
        self.assertEqual(report["status"], "warning")
        self.assertFalse(report["regressions"])

        over_limit = fixture(106)
        report = checker.compare(over_limit, baseline)
        self.assertEqual(report["status"], "failed")
        self.assertTrue(report["regressions"])

    def test_compares_every_operation_direction_and_flow_total(self):
        baseline = fixture()
        baseline["approval"] = {"status": "owner_approved"}
        measurement = fixture()
        measurement["flows"]["portal_projection"]["operations"]["attach"]["response"][
            "tokens"
        ] = 106
        report = checker.compare(measurement, baseline)
        self.assertEqual(len(report["regressions"]), 1)
        self.assertIn("portal_projection.operations.attach.response.tokens", report["regressions"][0]["path"])

    def test_fingerprint_drift_is_incompatible_not_a_regression(self):
        baseline = fixture()
        baseline["approval"] = {"status": "owner_approved"}
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
        baseline["approval"] = {"status": "owner_approved"}
        measurement = fixture()
        del measurement["flows"]["publish_to_widget"]["total"]["tokens"]
        report = checker.compare(measurement, baseline)
        self.assertEqual(report["status"], "baseline_incompatible")
        self.assertIn("missing or invalid integer metric", " ".join(report["incompatibilities"]))


if __name__ == "__main__":
    unittest.main()
