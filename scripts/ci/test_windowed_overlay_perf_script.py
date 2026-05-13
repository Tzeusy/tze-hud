#!/usr/bin/env python3
from __future__ import annotations

import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).parent / "windows" / "windowed-fullscreen-overlay-perf.ps1"


class WindowedOverlayPerfScriptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.script = SCRIPT_PATH.read_text(encoding="utf-8-sig")

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


if __name__ == "__main__":
    unittest.main()
