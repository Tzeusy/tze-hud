#!/usr/bin/env python3
from __future__ import annotations

import hashlib
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

    def _run(self, artifact_dir: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(SCRIPT_PATH), "--artifact-dir", str(artifact_dir)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_clean_artifact_matches_published_checksum(self) -> None:
        artifact_dir = self._fixture_dir()
        result = self._run(artifact_dir)
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


if __name__ == "__main__":
    unittest.main()
