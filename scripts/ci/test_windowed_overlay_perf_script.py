#!/usr/bin/env python3
from __future__ import annotations

import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).parent / "windows" / "windowed-fullscreen-overlay-perf.ps1"
POWERSHELL_TEST_PATH = (
    Path(__file__).parent / "windows" / "test-windowed-fullscreen-overlay-perf.ps1"
)
WORKFLOW_PATH = (
    Path(__file__).parents[2]
    / ".github"
    / "workflows"
    / "windowed-overlay-perf.yml"
)
CI_WORKFLOW_PATH = Path(__file__).parents[2] / ".github" / "workflows" / "ci.yml"


class WindowedOverlayPerfScriptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.script = SCRIPT_PATH.read_text(encoding="utf-8-sig")
        cls.powershell_test = POWERSHELL_TEST_PATH.read_text(encoding="utf-8-sig")
        cls.workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.ci_workflow = CI_WORKFLOW_PATH.read_text(encoding="utf-8")

    def test_waits_for_windows_subsystem_binary_and_reads_real_exit_code(self) -> None:
        self.assertIn("Start-Process", self.script)
        self.assertIn("-Wait", self.script)
        self.assertIn("-PassThru", self.script)
        self.assertIn("$process.ExitCode", self.script)
        self.assertNotIn("& $ExePath @args", self.script)
        self.assertNotIn("$LASTEXITCODE", self.script)

    def test_mode_failures_include_per_mode_logs(self) -> None:
        self.assertIn('$logDir = Join-Path $OutputDir "logs"', self.script)
        self.assertIn('$stdoutPath = Join-Path $logDir "$Mode.stdout.log"', self.script)
        self.assertIn('$stderrPath = Join-Path $logDir "$Mode.stderr.log"', self.script)
        self.assertIn("stdout=$stdoutPath stderr=$stderrPath", self.script)

    def test_default_launch_preserves_overlay_monitor_auto_size(self) -> None:
        self.assertNotIn('[int]$Width = 1920', self.script)
        self.assertNotIn('[int]$Height = 1080', self.script)
        self.assertIn('$surfaceArgs = @()', self.script)
        self.assertNotIn('"-Width",', self.workflow)
        self.assertNotIn('"-Height",', self.workflow)

    def test_effective_surface_validation_precedes_delta_calculation(self) -> None:
        validation = self.script.index("$effectiveSurface = Assert-ComparableEffectiveSurfaces")
        delta = self.script.index("$deltaP50 =")
        self.assertLess(validation, delta)
        self.assertIn("Get-EffectiveSurfaceDimensions", self.script)
        self.assertIn("[uint32]::TryParse", self.script)
        self.assertIn("[System.Type]::GetTypeCode", self.script)
        self.assertIn("effective surface mismatch", self.script)
        self.assertIn("effective_surface = [ordered]@{", self.script)

    def test_native_powershell_contract_tests_cover_invalid_surfaces(self) -> None:
        self.assertIn("missing window object", self.powershell_test)
        self.assertIn("missing window.width", self.powershell_test)
        self.assertIn("missing window.height", self.powershell_test)
        self.assertIn("malformed window.width", self.powershell_test)
        self.assertIn("malformed window.height", self.powershell_test)
        self.assertIn('Width "3840"', self.powershell_test)
        self.assertIn("effective surface mismatch", self.powershell_test)
        self.assertIn("test-windowed-fullscreen-overlay-perf.ps1", self.ci_workflow)

    def test_locked_overlay_delta_budget_is_unchanged(self) -> None:
        self.assertIn('[int]$TargetDeltaUs = 500', self.script)
        self.assertIn('$passesTarget = $deltaP99 -le $TargetDeltaUs', self.script)


if __name__ == "__main__":
    unittest.main()
