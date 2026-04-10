#!/usr/bin/env python3
import os
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path
from typing import Tuple


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_PATH = REPO_ROOT / "scripts" / "epic-report-scaffold.sh"


class EpicReportScaffoldTests(unittest.TestCase):
    def _write_executable(self, path: Path, content: str) -> None:
        path.write_text(content, encoding="utf-8")
        mode = path.stat().st_mode
        path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    def _run_scaffold(self, show_json: str, children_json: str) -> Tuple[subprocess.CompletedProcess, Path]:
        temp_dir_obj = tempfile.TemporaryDirectory(prefix="epic-report-scaffold-")
        self.addCleanup(temp_dir_obj.cleanup)
        temp_dir = Path(temp_dir_obj.name)
        repo_dir = temp_dir / "repo"
        repo_dir.mkdir(parents=True, exist_ok=True)

        subprocess.run(["git", "init"], cwd=repo_dir, check=True, capture_output=True, text=True)

        show_json_file = temp_dir / "show.json"
        children_json_file = temp_dir / "children.json"
        show_json_file.write_text(show_json, encoding="utf-8")
        children_json_file.write_text(children_json, encoding="utf-8")

        mock_bin = temp_dir / "bin"
        mock_bin.mkdir(parents=True, exist_ok=True)

        self._write_executable(
            mock_bin / "bd",
            textwrap.dedent(
                """\
                #!/usr/bin/env bash
                set -euo pipefail
                cmd="${1:-}"
                shift || true
                case "$cmd" in
                  show)
                    cat "${BD_SHOW_JSON_FILE}"
                    ;;
                  children)
                    cat "${BD_CHILDREN_JSON_FILE}"
                    ;;
                  *)
                    echo "unsupported mock bd subcommand: $cmd" >&2
                    exit 1
                    ;;
                esac
                """
            ),
        )

        env = os.environ.copy()
        env["PATH"] = f"{mock_bin}:{env['PATH']}"
        env["BD_SHOW_JSON_FILE"] = str(show_json_file)
        env["BD_CHILDREN_JSON_FILE"] = str(children_json_file)

        proc = subprocess.run(
            ["bash", str(SCRIPT_PATH), "hud-test", str(repo_dir)],
            capture_output=True,
            text=True,
            env=env,
        )
        return proc, repo_dir

    def test_accepts_object_root_from_bd_show(self) -> None:
        show_json = '{"id":"hud-test","title":"Object Shape Epic","status":"open","description":"obj root","issue_type":"epic","priority":1}'
        children_json = '[{"id":"hud-test.1","title":"child","status":"closed","issue_type":"task"}]'
        proc, repo_dir = self._run_scaffold(show_json, children_json)

        self.assertEqual(proc.returncode, 0, msg=f"stderr:\n{proc.stderr}\nstdout:\n{proc.stdout}")
        report_file = repo_dir / "docs" / "reports" / "hud-test-object-shape-epic.md"
        self.assertTrue(report_file.exists(), "expected report file not generated for object root")
        report_content = report_file.read_text(encoding="utf-8")
        self.assertIn("# Epic Report: Object Shape Epic", report_content)
        self.assertIn("**Status**: 1/1 children closed (open)", report_content)

    def test_accepts_array_root_from_bd_show(self) -> None:
        show_json = '[{"id":"hud-test","title":"Array Shape Epic","status":"open","description":"array root","issue_type":"epic","priority":2}]'
        children_json = '[{"id":"hud-test.1","title":"child","status":"open","issue_type":"task"}]'
        proc, repo_dir = self._run_scaffold(show_json, children_json)

        self.assertEqual(proc.returncode, 0, msg=f"stderr:\n{proc.stderr}\nstdout:\n{proc.stdout}")
        report_file = repo_dir / "docs" / "reports" / "hud-test-array-shape-epic.md"
        self.assertTrue(report_file.exists(), "expected report file not generated for array root")
        report_content = report_file.read_text(encoding="utf-8")
        self.assertIn("# Epic Report: Array Shape Epic", report_content)
        self.assertIn("**Status**: 0/1 children closed (open)", report_content)

    def test_rejects_empty_array_root(self) -> None:
        show_json = "[]"
        children_json = "[]"
        proc, _ = self._run_scaffold(show_json, children_json)
        self.assertNotEqual(proc.returncode, 0, "expected failure for empty bd show array root")
        self.assertIn("ERROR: Could not parse epic payload for hud-test", proc.stdout + proc.stderr)

    def test_escapes_markdown_table_cells(self) -> None:
        show_json = '{"id":"hud-test","title":"Escape Check","status":"open","description":"desc","issue_type":"epic","priority":1}'
        children_json = '[{"id":"hud|test.1","title":"line1\\nline2|x","status":"open","issue_type":"task"}]'
        proc, repo_dir = self._run_scaffold(show_json, children_json)

        self.assertEqual(proc.returncode, 0, msg=f"stderr:\n{proc.stderr}\nstdout:\n{proc.stdout}")
        report_file = repo_dir / "docs" / "reports" / "hud-test-escape-check.md"
        report_content = report_file.read_text(encoding="utf-8")
        self.assertIn("| hud\\|test.1 | line1 line2\\|x | open | - | task |", report_content)

    def test_rejects_overwrite_of_existing_report(self) -> None:
        show_json = '{"id":"hud-test","title":"Repeat Run","status":"open","description":"desc","issue_type":"epic","priority":1}'
        children_json = "[]"
        first_proc, repo_dir = self._run_scaffold(show_json, children_json)
        self.assertEqual(first_proc.returncode, 0, msg=f"stderr:\n{first_proc.stderr}\nstdout:\n{first_proc.stdout}")
        temp_dir = repo_dir.parent
        env = os.environ.copy()
        env["PATH"] = f"{temp_dir / 'bin'}:{env['PATH']}"
        env["BD_SHOW_JSON_FILE"] = str(temp_dir / "show.json")
        env["BD_CHILDREN_JSON_FILE"] = str(temp_dir / "children.json")

        second_proc = subprocess.run(
            ["bash", str(SCRIPT_PATH), "hud-test", str(repo_dir)],
            capture_output=True,
            text=True,
            env=env,
        )
        self.assertNotEqual(second_proc.returncode, 0)
        self.assertIn("ERROR: Report file already exists", second_proc.stdout + second_proc.stderr)

    def test_falls_back_to_epic_id_when_slug_is_empty(self) -> None:
        show_json = '{"id":"hud-test","title":"!!!","status":"open","description":"desc","issue_type":"epic","priority":1}'
        children_json = "[]"
        proc, repo_dir = self._run_scaffold(show_json, children_json)
        self.assertEqual(proc.returncode, 0, msg=f"stderr:\n{proc.stderr}\nstdout:\n{proc.stdout}")
        report_file = repo_dir / "docs" / "reports" / "hud-test-hud-test.md"
        self.assertTrue(report_file.exists(), "expected slug fallback to use epic ID")


if __name__ == "__main__":
    unittest.main()
