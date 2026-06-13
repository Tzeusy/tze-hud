from __future__ import annotations

import sys
import tempfile
import unittest
import uuid
from pathlib import Path
from unittest import mock


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))
sys.path.insert(0, str(SCRIPT_DIR / "proto_gen"))

import windows_media_ingress_exemplar as media  # noqa: E402


class WindowsMediaIngressExemplarTests(unittest.TestCase):
    def test_video_only_sdp_offer_has_no_audio_track(self) -> None:
        offer = media.build_video_only_sdp_offer(
            stream_id=uuid.UUID("11111111-2222-7333-8444-555555555555"),
            source_label="synthetic-color-bars",
            width=640,
            height=360,
            fps=30,
        ).decode("utf-8")

        self.assertIn("m=video 9 UDP/TLS/RTP/SAVPF 96", offer)
        self.assertIn("a=sendonly", offer)
        self.assertIn("a=rtpmap:96 H264/90000", offer)
        self.assertIn("a=framesize:96 640-360", offer)
        self.assertNotIn("m=audio", offer)
        self.assertNotIn("AUDIO_OPUS", offer)

    def test_youtube_evidence_uses_official_embed_url(self) -> None:
        html = media.build_source_evidence_html()

        self.assertIn("https://www.youtube.com/embed/O0FGCxkHM-U", html)
        self.assertIn('id="youtube-source-evidence"', html)
        self.assertIn("strict-origin-when-cross-origin", html)
        for banned in media.BANNED_SOURCE_MARKERS:
            self.assertNotIn(banned, html.lower())

    def test_policy_review_blocks_raw_youtube_frame_bridge(self) -> None:
        review = media.policy_review()

        self.assertEqual(review["youtube_video_id"], "O0FGCxkHM-U")
        self.assertEqual(
            review["raw_youtube_frame_bridge"],
            "blocked_pending_policy_approval",
        )
        self.assertEqual(review["audio_route_to_hud"], "none")
        self.assertIn("self-owned/local", review["hud_ingress_source"])

    def test_rejects_invalid_youtube_video_id_before_launch(self) -> None:
        with self.assertRaisesRegex(ValueError, "YouTube id format"):
            media.validate_youtube_video_id("O0FGCxkHM-U'; calc; '")

    def test_rejects_unapproved_media_zone(self) -> None:
        with self.assertRaisesRegex(ValueError, "approved zone"):
            media.validate_approved_media_zone("desktop")

    def test_local_youtube_sidecar_launches_generated_html(self) -> None:
        with tempfile.TemporaryDirectory() as tmpdir:
            args = media.build_parser().parse_args(
                [
                    "youtube-sidecar",
                    "--output-dir",
                    tmpdir,
                ]
            )
            with mock.patch.object(media.webbrowser, "open", return_value=True) as opened:
                evidence = media.launch_youtube_sidecar(args)

        opened.assert_called_once()
        opened_url = opened.call_args.args[0]
        self.assertTrue(opened_url.startswith("file://"), opened_url)
        self.assertEqual(evidence["official_player_url"], media.YOUTUBE_EMBED_URL)
        self.assertTrue(Path(evidence["html_evidence_path"]).name.endswith(".html"))


class ValidateOfficialPlayerFramesTests(unittest.TestCase):
    """Tests for _validate_official_player_frames — headless numeric fixtures only."""

    # -----------------------------------------------------------------------
    # Fixtures based on observed hud-aaf39 bug evidence:
    # Error-153 page: mean_rgb ~[38.95, 41.52, 44.42] on every frame,
    # with only blinking cursor/spinner making hashes differ.
    # Real playback:  frames swing from ~[46, 32, 22] to ~[7, 7, 9].
    # -----------------------------------------------------------------------

    _ERROR_PAGE_RGB = [38.95, 41.52, 44.42]

    def _hash(self, tag: str) -> str:
        import hashlib
        return hashlib.sha256(tag.encode()).hexdigest()

    # --- Error-page cases (must REJECT) ------------------------------------

    def test_error_page_static_frames_rejected(self) -> None:
        """Near-static dark frames (Error-153 evidence) must not validate."""
        # Two frames with identical mean_rgb; hashes differ (blinking cursor).
        frames = [
            {"mean_rgb": list(self._ERROR_PAGE_RGB), "sha256": self._hash("frame-0")},
            {"mean_rgb": list(self._ERROR_PAGE_RGB), "sha256": self._hash("frame-1")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertFalse(result["capture_validated"], result)
        self.assertIsNotNone(result["rejection_reason"])
        self.assertIn("static", result["rejection_reason"])

    def test_error_page_tiny_variance_rejected(self) -> None:
        """Frames with sub-threshold luminance variance (< 1.0 stddev) are rejected."""
        # Vary by ±0.2 luma — well below the 1.0 threshold.
        frames = [
            {"mean_rgb": [38.9, 41.5, 44.4], "sha256": self._hash("a")},
            {"mean_rgb": [39.1, 41.6, 44.5], "sha256": self._hash("b")},
            {"mean_rgb": [38.95, 41.52, 44.42], "sha256": self._hash("c")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertFalse(result["capture_validated"], result)
        self.assertLess(result["luma_stddev"], 1.0)

    def test_no_frames_rejected(self) -> None:
        result = media._validate_official_player_frames([])
        self.assertFalse(result["capture_validated"])
        self.assertEqual(result["frame_count"], 0)
        self.assertIn("no frames", result["rejection_reason"])

    def test_pure_black_frames_rejected(self) -> None:
        frames = [
            {"mean_rgb": [0.0, 0.0, 0.0], "sha256": self._hash("black-0")},
            {"mean_rgb": [0.0, 0.0, 0.1], "sha256": self._hash("black-1")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertFalse(result["capture_validated"])
        self.assertIn("blank", result["rejection_reason"])

    def test_single_unique_hash_rejected(self) -> None:
        """Frozen frame (single hash repeated) is rejected even if not black."""
        frozen_hash = self._hash("frozen")
        frames = [
            {"mean_rgb": [46.0, 32.0, 22.0], "sha256": frozen_hash},
            {"mean_rgb": [46.0, 32.0, 22.0], "sha256": frozen_hash},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertFalse(result["capture_validated"])
        self.assertIn("frozen", result["rejection_reason"])

    # --- Real-playback cases (must ACCEPT) ---------------------------------

    def test_real_playback_frames_accepted(self) -> None:
        """Frames with large inter-frame luminance swings must validate."""
        # Evidence from hud-3ervz: [46,32,22] → [7,7,9].
        frames = [
            {"mean_rgb": [46.0, 32.0, 22.0], "sha256": self._hash("play-0")},
            {"mean_rgb": [7.0, 7.0, 9.0], "sha256": self._hash("play-1")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertTrue(result["capture_validated"], result)
        self.assertIsNone(result["rejection_reason"])
        self.assertGreaterEqual(result["luma_stddev"], 1.0)

    def test_moderate_variance_accepted(self) -> None:
        """Frames with exactly-threshold-crossing variance are accepted."""
        # Design frames so luma stddev is just above 1.0.
        # frame0 luma ≈ 0.299*50+0.587*50+0.114*50 = 50; frame1 luma ≈ 47.
        # stddev of [50, 47] = 1.5 > 1.0.
        frames = [
            {"mean_rgb": [50.0, 50.0, 50.0], "sha256": self._hash("mod-0")},
            {"mean_rgb": [47.0, 47.0, 47.0], "sha256": self._hash("mod-1")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertTrue(result["capture_validated"], result)

    def test_three_varied_frames_accepted(self) -> None:
        """Three frames with meaningful variance pass all checks."""
        frames = [
            {"mean_rgb": [120.0, 80.0, 40.0], "sha256": self._hash("v-0")},
            {"mean_rgb": [80.0, 120.0, 80.0], "sha256": self._hash("v-1")},
            {"mean_rgb": [40.0, 40.0, 120.0], "sha256": self._hash("v-2")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertTrue(result["capture_validated"], result)
        self.assertEqual(result["frame_count"], 3)
        self.assertEqual(result["distinct_hashes"], 3)

    # --- Output-shape invariants -------------------------------------------

    def test_result_always_has_required_keys(self) -> None:
        """All result dicts must expose the same top-level keys."""
        required = {
            "capture_validated",
            "frame_count",
            "distinct_hashes",
            "mean_luma",
            "luma_stddev",
            "rejection_reason",
        }
        # Test with the error-page fixture (rejected path).
        frames = [
            {"mean_rgb": list(self._ERROR_PAGE_RGB), "sha256": self._hash("k-0")},
            {"mean_rgb": list(self._ERROR_PAGE_RGB), "sha256": self._hash("k-1")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertTrue(required.issubset(result.keys()), result.keys())

    def test_accepted_result_has_null_rejection_reason(self) -> None:
        frames = [
            {"mean_rgb": [46.0, 32.0, 22.0], "sha256": self._hash("ok-0")},
            {"mean_rgb": [7.0, 7.0, 9.0], "sha256": self._hash("ok-1")},
        ]
        result = media._validate_official_player_frames(frames)
        self.assertIsNone(result["rejection_reason"])


if __name__ == "__main__":
    unittest.main()
