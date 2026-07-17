#!/usr/bin/env python3
"""Offline contract tests for the startup-atomic Windows quiescent harness."""

from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).parents[2]
SCRIPT_PATH = ROOT / "scripts" / "ci" / "windows" / "run-quiescent-efficiency.ps1"
CI_WORKFLOW_PATH = ROOT / ".github" / "workflows" / "ci.yml"
JUSTFILE_PATH = ROOT / "justfile"


class QuiescentEfficiencyLaunchContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.script = SCRIPT_PATH.read_text(encoding="utf-8-sig")
        cls.workflow = CI_WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.justfile = JUSTFILE_PATH.read_text(encoding="utf-8")

    def test_applies_affinity_while_creating_the_measured_application(self) -> None:
        self.assertIn('$affinityMask = "3"', self.script)
        self.assertIn('start "" /b /wait /affinity {0} {1} {2}', self.script)
        self.assertIn("$affinityMask, $quotedExePath", self.script)
        self.assertIn("-FilePath $env:ComSpec", self.script)
        self.assertNotIn("-FilePath $ExePath", self.script)
        self.assertNotIn(".ProcessorAffinity", self.script)

    def test_fast_ci_runs_the_startup_atomicity_contract(self) -> None:
        self.assertIn(
            "python3 scripts/ci/test_run_quiescent_efficiency_script.py",
            self.workflow,
        )
        self.assertIn(
            "python3 scripts/ci/test_run_quiescent_efficiency_script.py",
            self.justfile,
        )

    def test_keeps_the_runtime_artifact_as_the_final_affinity_observer(self) -> None:
        self.assertIn("--quiescent-efficiency-emit", self.script)
        self.assertIn("--require-constrained", self.script)
        self.assertIn("--require-window-mode overlay", self.script)


if __name__ == "__main__":
    unittest.main()
