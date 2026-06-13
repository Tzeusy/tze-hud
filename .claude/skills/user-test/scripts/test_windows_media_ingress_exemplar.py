import unittest
import asyncio
import shutil
from pathlib import Path

import windows_media_ingress_exemplar as exemplar


class WindowsMediaIngressExemplarTests(unittest.TestCase):
    def test_policy_review_names_approved_bridge_boundary(self):
        review = exemplar.policy_review()

        self.assertEqual(
            review["raw_youtube_frame_bridge"],
            "approved_operator_visible_player_frame_bridge",
        )
        self.assertEqual(
            review["bridge_path_name"],
            "operator-visible-official-player-window-capture-to-media-ingress-open",
        )
        self.assertEqual(review["audio_route_to_hud"], "none")
        self.assertIn("yt-dlp", review["prohibited_paths"])
        self.assertIn("cache", review["prohibited_paths"])

    def test_source_evidence_html_uses_official_player_and_keeps_controls_available(self):
        html = exemplar.build_source_evidence_html(exemplar.YOUTUBE_VIDEO_ID)

        self.assertIn(
            f"https://www.youtube.com/embed/{exemplar.YOUTUBE_VIDEO_ID}",
            html,
        )
        self.assertIn("allowfullscreen", html)
        self.assertNotIn("controls=0", html)
        self.assertNotIn("noreferrer", html)

    def test_sidecar_evidence_keeps_relative_output_path_relative(self):
        output_dir = Path("build/test-windows-media-ingress-sidecar")
        shutil.rmtree(output_dir, ignore_errors=True)
        try:
            args = exemplar.build_parser().parse_args(
                ["youtube-sidecar", "--dry-run", "--output-dir", str(output_dir)]
            )
            evidence = exemplar.launch_youtube_sidecar(args)

            self.assertEqual(
                evidence["html_evidence_path"],
                str(output_dir / "youtube_source_evidence.html"),
            )
        finally:
            shutil.rmtree(output_dir, ignore_errors=True)

    def test_bridge_dry_run_evidence_does_not_claim_live_frames(self):
        sidecar = {
            "video_id": exemplar.YOUTUBE_VIDEO_ID,
            "official_player_url": exemplar.YOUTUBE_EMBED_URL,
        }
        evidence = exemplar.build_youtube_bridge_dry_run_evidence(
            sidecar_evidence=sidecar,
            target="example.invalid:50051",
            agent_id=exemplar.YOUTUBE_BRIDGE_AGENT_ID,
            zone_name=exemplar.APPROVED_MEDIA_ZONE,
        )

        self.assertEqual(evidence["media_ingress_entrypoint"], "MediaIngressOpen")
        self.assertFalse(evidence["media_ingress_open_attempted"])
        self.assertFalse(evidence["media_ingress_open_admitted"])
        self.assertFalse(evidence["hud_runtime_receives_youtube_frames"])
        self.assertEqual(evidence["download_or_extraction"], "not_used")
        self.assertEqual(evidence["cache_or_offline_copy"], "not_used")

    def test_youtube_bridge_parser_defaults_to_bridge_agent(self):
        args = exemplar.build_parser().parse_args(
            ["youtube-bridge", "--dry-run", "--media-ingress-dry-run"]
        )

        self.assertEqual(args.command, "youtube-bridge")
        self.assertEqual(args.agent_id, exemplar.YOUTUBE_BRIDGE_AGENT_ID)
        self.assertEqual(args.source_label, exemplar.YOUTUBE_BRIDGE_SOURCE_LABEL)
        self.assertEqual(args.zone_name, exemplar.APPROVED_MEDIA_ZONE)

    def test_youtube_bridge_live_path_fails_until_capture_adapter_exists(self):
        args = exemplar.build_parser().parse_args(["youtube-bridge", "--dry-run"])

        with self.assertRaisesRegex(RuntimeError, "frame-capture adapter"):
            asyncio.run(exemplar.run_youtube_bridge(args))

    def test_invalid_approved_zone_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "approved zone"):
            exemplar.validate_approved_media_zone("pip")


if __name__ == "__main__":
    unittest.main()
