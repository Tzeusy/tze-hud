#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).parent / "release_artifact_provenance_smoke.sh"


class ReleaseArtifactProvenanceSmokeTests(unittest.TestCase):
    def _fixture_dir(self, payload: bytes = b"release executable fixture\n") -> Path:
        temp_dir = Path(tempfile.mkdtemp(prefix="tze-hud-release-provenance."))
        self.addCleanup(shutil.rmtree, temp_dir, ignore_errors=True)
        exe_path = temp_dir / "tze_hud.exe"
        exe_path.write_bytes(payload)
        digest = hashlib.sha256(payload).hexdigest()
        (temp_dir / "tze_hud.exe.sha256").write_text(
            f"{digest}  tze_hud.exe\n",
            encoding="utf-8",
        )
        return temp_dir

    def _fake_objdump(self, *, program_headers: str, section_headers: str) -> Path:
        tool_dir = Path(tempfile.mkdtemp(prefix="tze-hud-fake-objdump."))
        self.addCleanup(shutil.rmtree, tool_dir, ignore_errors=True)
        tool_path = tool_dir / "objdump"
        tool_path.write_text(
            "#!/usr/bin/env python3\n"
            "import sys\n"
            f"program_headers = {program_headers!r}\n"
            f"section_headers = {section_headers!r}\n"
            "if sys.argv[1] == '-p':\n"
            "    print(program_headers, end='')\n"
            "elif sys.argv[1] == '-h':\n"
            "    print(section_headers, end='')\n"
            "else:\n"
            "    raise SystemExit(f'unexpected objdump arguments: {sys.argv[1:]}')\n",
            encoding="utf-8",
        )
        tool_path.chmod(0o755)
        return tool_path

    def _run(
        self,
        artifact_dir: Path,
        *,
        objdump: Path | None = None,
    ) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        if objdump is not None:
            environment["OBJDUMP"] = str(objdump)
        return subprocess.run(
            ["bash", str(SCRIPT_PATH), "--artifact-dir", str(artifact_dir)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
            check=False,
        )

    def test_clean_artifact_matches_published_checksum(self) -> None:
        artifact_dir = self._fixture_dir()
        objdump = self._fake_objdump(
            program_headers=(
                "The Data Directory\n"
                "Entry 2 0000000000001000 00000080 Resource Directory [.rsrc]\n"
            ),
            section_headers="Idx Name          Size\n  4 .rsrc         00000080\n",
        )
        result = self._run(artifact_dir, objdump=objdump)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("pass", result.stdout)

    def test_checksum_mismatch_fails_gate(self) -> None:
        artifact_dir = self._fixture_dir()
        (artifact_dir / "tze_hud.exe").write_bytes(b"tampered executable fixture\n")
        result = self._run(artifact_dir)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum mismatch", result.stderr)

    def test_checksum_must_reference_deterministic_artifact_name(self) -> None:
        artifact_dir = self._fixture_dir()
        checksum_path = artifact_dir / "tze_hud.exe.sha256"
        digest = checksum_path.read_text(encoding="utf-8").split()[0]
        checksum_path.write_text(f"{digest}  ./tze_hud.exe\n", encoding="utf-8")
        result = self._run(artifact_dir)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must reference tze_hud.exe", result.stderr)

    def test_empty_checksum_reports_diagnostic(self) -> None:
        artifact_dir = self._fixture_dir()
        (artifact_dir / "tze_hud.exe.sha256").write_text("", encoding="utf-8")
        result = self._run(artifact_dir)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checksum file is empty", result.stderr)

    def test_malformed_checksum_reports_diagnostic(self) -> None:
        artifact_dir = self._fixture_dir()
        (artifact_dir / "tze_hud.exe.sha256").write_text(
            "not-a-sha256  tze_hud.exe\n",
            encoding="utf-8",
        )
        result = self._run(artifact_dir)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("malformed SHA-256 digest", result.stderr)

    def test_requires_nonzero_pe_resource_directory(self) -> None:
        artifact_dir = self._fixture_dir()
        objdump = self._fake_objdump(
            program_headers=(
                "The Data Directory\n"
                "Entry 2 0000000000000000 00000000 Resource Directory [.rsrc]\n"
            ),
            section_headers="Idx Name          Size\n  4 .rsrc         00000100\n",
        )
        result = self._run(artifact_dir, objdump=objdump)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("nonzero PE resource directory", result.stderr)

    def test_requires_pe_resource_directory_entry(self) -> None:
        artifact_dir = self._fixture_dir()
        objdump = self._fake_objdump(
            program_headers="The Data Directory\n",
            section_headers="Idx Name          Size\n  4 .rsrc         00000100\n",
        )
        result = self._run(artifact_dir, objdump=objdump)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing PE resource directory", result.stderr)

    def test_requires_nonempty_rsrc_section(self) -> None:
        artifact_dir = self._fixture_dir()
        objdump = self._fake_objdump(
            program_headers=(
                "The Data Directory\n"
                "Entry 2 0000000000001000 00000080 Resource Directory [.rsrc]\n"
            ),
            section_headers="Idx Name          Size\n  4 .rsrc         00000000\n",
        )
        result = self._run(artifact_dir, objdump=objdump)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("nonempty .rsrc section", result.stderr)

    def test_nonzero_pe_resource_directory_and_rsrc_section_pass(self) -> None:
        artifact_dir = self._fixture_dir()
        objdump = self._fake_objdump(
            program_headers=(
                "The Data Directory\n"
                "Entry 2 0000000000001000 00000080 Resource Directory [.rsrc]\n"
            ),
            section_headers="Idx Name          Size\n  4 .rsrc         00000080\n",
        )
        result = self._run(artifact_dir, objdump=objdump)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
