#!/usr/bin/env python3
"""Regression tests for quickstart MCP client-config emission."""

from __future__ import annotations

import json
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
QUICKSTART = REPO_ROOT / "scripts" / "quickstart.sh"


class QuickstartMcpConfigTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir_obj = tempfile.TemporaryDirectory(prefix="quickstart-mcp-config-")
        self.temp_dir = Path(self.temp_dir_obj.name)

    def tearDown(self) -> None:
        self.temp_dir_obj.cleanup()

    def run_quickstart(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                "bash",
                str(QUICKSTART),
                "--config",
                str(self.temp_dir / "tze_hud.toml"),
                "--psk-file",
                str(self.temp_dir / "tze_hud.psk"),
                "--bin",
                "/bin/true",
                *args,
            ],
            cwd=self.temp_dir,
            capture_output=True,
            text=True,
            check=False,
        )

    @staticmethod
    def emitted_json(stdout: str) -> dict[str, object]:
        return json.loads(stdout)

    def test_bare_option_emits_wired_ready_to_merge_json(self) -> None:
        secret = "operator-secret"

        result = self.run_quickstart(
            "--psk",
            secret,
            "--host",
            "hud.test",
            "--mcp-port",
            "9191",
            "--emit-mcp-config",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        config = self.emitted_json(result.stdout)
        server = config["mcpServers"]["tze-hud-runtime"]
        self.assertEqual(server["type"], "url")
        self.assertEqual(server["url"], "http://hud.test:9191/mcp")
        self.assertEqual(
            server["headers"]["Authorization"], f"Bearer {secret}"
        )

    def test_path_option_writes_mode_0600_without_printing_secret(self) -> None:
        secret = "file-only-secret"
        output_path = self.temp_dir / "tze-hud.mcp.json"

        result = self.run_quickstart(
            "--psk",
            secret,
            "--print-attach-info",
            f"--emit-mcp-config={output_path}",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(secret, result.stdout)
        self.assertNotIn(secret, result.stderr)
        config = json.loads(output_path.read_text(encoding="utf-8"))
        authorization = config["mcpServers"]["tze-hud-runtime"]["headers"][
            "Authorization"
        ]
        self.assertEqual(authorization, f"Bearer {secret}")
        mode = stat.S_IMODE(output_path.stat().st_mode)
        self.assertEqual(mode, 0o600)

    def test_path_option_is_emission_only_and_needs_no_binary(self) -> None:
        output_path = self.temp_dir / "headless.mcp.json"

        result = self.run_quickstart(
            "--psk",
            "headless-file-secret",
            "--bin",
            str(self.temp_dir / "missing-tze-hud"),
            f"--emit-mcp-config={output_path}",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(output_path.is_file())
        self.assertNotIn("No usable tze_hud binary", result.stderr)

    def test_redacted_headless_mode_rejects_bare_secret_emission(self) -> None:
        secret = "must-not-be-printed"

        result = self.run_quickstart(
            "--psk",
            secret,
            "--print-attach-info",
            "--emit-mcp-config",
        )

        self.assertEqual(result.returncode, 1)
        self.assertNotIn(secret, result.stdout)
        self.assertNotIn(secret, result.stderr)
        self.assertIn("--emit-mcp-config=<path>", result.stderr)

    def test_emitted_json_escapes_operator_provided_psk(self) -> None:
        secret = 'quote" slash\\ and control\x01 remain valid'

        result = self.run_quickstart(
            "--psk",
            secret,
            "--emit-mcp-config",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        config = self.emitted_json(result.stdout)
        authorization = config["mcpServers"]["tze-hud-runtime"]["headers"][
            "Authorization"
        ]
        self.assertEqual(authorization, f"Bearer {secret}")

    def test_path_option_preserves_existing_client_config(self) -> None:
        output_path = self.temp_dir / "existing.json"
        original = '{"keep": true}\n'
        output_path.write_text(original, encoding="utf-8")

        result = self.run_quickstart(
            "--psk",
            "replacement-secret",
            f"--emit-mcp-config={output_path}",
        )

        self.assertEqual(result.returncode, 1)
        self.assertEqual(output_path.read_text(encoding="utf-8"), original)
        self.assertIn("refusing to overwrite", result.stderr)


if __name__ == "__main__":
    unittest.main()
