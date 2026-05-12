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
        self.assertIn("tze_hud YouTube source evidence O0FGCxkHM-U", html)
        self.assertIn('id="youtube-source-evidence"', html)
        self.assertIn("strict-origin-when-cross-origin", html)
        for banned in media.BANNED_SOURCE_MARKERS:
            self.assertNotIn(banned, html.lower())

    def test_policy_review_names_approved_frame_bridge(self) -> None:
        review = media.policy_review()

        self.assertEqual(review["youtube_video_id"], "O0FGCxkHM-U")
        self.assertEqual(
            review["raw_youtube_frame_bridge"],
            "approved_operator_visible_player_frame_bridge",
        )
        self.assertEqual(
            review["bridge_path_name"],
            "operator-visible-official-player-window-capture-to-media-ingress-open",
        )
        self.assertEqual(review["audio_route_to_hud"], "none")
        self.assertIn("official-player raw-frame bridge", review["hud_ingress_source"])

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


if __name__ == "__main__":
    unittest.main()
