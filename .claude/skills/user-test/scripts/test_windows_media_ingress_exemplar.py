import unittest
import asyncio
import json
import tempfile
from pathlib import Path

import windows_media_ingress_exemplar as exemplar


def valid_frame_capture_fixture() -> dict:
    return {
        "adapter": exemplar.YOUTUBE_FRAME_CAPTURE_ADAPTER,
        "video_id": exemplar.YOUTUBE_VIDEO_ID,
        "official_player_url": exemplar.YOUTUBE_EMBED_URL,
        "capture_surface": "operator-visible official YouTube player window",
        "capture_api": "fixture",
        "operator_visible_player_controls": True,
        "download_or_extraction": "not_used",
        "cache_or_offline_copy": "not_used",
        "audio_route_to_hud": "none",
        "saved_frame_files": [],
        "window": {
            "title": "tze_hud YouTube source evidence - YouTube",
            "left": 10,
            "top": 20,
            "width": 960,
            "height": 540,
        },
        "selected_window_visible_area": 960 * 540,
        "selected_window_moved_to_primary": True,
        "playback_click_sent": True,
        "captured_frames": [
            {
                "index": 0,
                "sha256": "a" * 64,
                "png_bytes": 1200,
                "sampled_pixels": 64,
                "mean_rgb": [12.0, 20.0, 30.0],
            },
            {
                "index": 1,
                "sha256": "b" * 64,
                "png_bytes": 1210,
                "sampled_pixels": 64,
                "mean_rgb": [14.0, 24.0, 38.0],
            },
        ],
    }


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
        self.assertIn(
            f"tze_hud YouTube source evidence {exemplar.YOUTUBE_VIDEO_ID}",
            html,
        )
        self.assertIn("allowfullscreen", html)
        self.assertIn("autoplay=1", html)
        self.assertIn("mute=1", html)
        self.assertNotIn("controls=0", html)
        self.assertNotIn("noreferrer", html)

    def test_frame_capture_window_match_uses_controlled_title_for_selected_video(self):
        script = exemplar.build_windows_frame_capture_powershell(
            video_id="abcdefghijk",
            sample_count=1,
            sample_interval_s=0.0,
            settle_s=0.0,
        )

        self.assertIn("tze_hud YouTube source evidence $VideoId", script)
        self.assertIn("$title.StartsWith($ExpectedTitlePrefix", script)
        self.assertIn("Get-VisibleAreaOnPrimary", script)
        self.assertIn("selected_window_visible_area", script)
        self.assertIn("SetWindowPos", script)
        self.assertIn("SetCursorPos", script)
        self.assertIn("playback_click_sent", script)
        self.assertIn("visible_area", script)
        self.assertNotIn("Sort-Object area -Descending", script)
        self.assertNotIn("O0FGCxkHM-U", script)
        self.assertNotIn("YouTube|", script)

    def test_sidecar_evidence_keeps_relative_output_path_relative(self):
        with tempfile.TemporaryDirectory(dir=".") as tmpdir:
            output_dir = Path(tmpdir) / "sidecar"
            args = exemplar.build_parser().parse_args(
                ["youtube-sidecar", "--dry-run", "--output-dir", str(output_dir)]
            )
            evidence = exemplar.launch_youtube_sidecar(args)

            self.assertEqual(
                evidence["html_evidence_path"],
                str(output_dir / "youtube_source_evidence.html"),
            )

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
        self.assertFalse(evidence["captured_youtube_frames_available_to_bridge"])

    def test_frame_capture_fixture_validates_official_player_boundary(self):
        evidence = exemplar.validate_frame_capture_evidence(valid_frame_capture_fixture())

        self.assertTrue(evidence["capture_validated"])
        self.assertEqual(evidence["captured_frame_count"], 2)
        self.assertEqual(evidence["distinct_frame_hashes"], 2)
        self.assertEqual(evidence["download_or_extraction"], "not_used")
        self.assertEqual(evidence["cache_or_offline_copy"], "not_used")
        self.assertEqual(evidence["audio_route_to_hud"], "none")

    def test_frame_capture_fixture_rejects_cached_frames(self):
        fixture = valid_frame_capture_fixture()
        fixture["saved_frame_files"] = ["C:/temp/frame.png"]

        with self.assertRaisesRegex(RuntimeError, "persist captured frame"):
            exemplar.validate_frame_capture_evidence(fixture)

    def test_frame_capture_fixture_rejects_wrong_official_player_url(self):
        fixture = valid_frame_capture_fixture()
        fixture["official_player_url"] = "https://example.invalid/not-youtube"

        with self.assertRaisesRegex(RuntimeError, "official player URL"):
            exemplar.validate_frame_capture_evidence(fixture)

    def test_frame_capture_fixture_rejects_malformed_mean_rgb(self):
        fixture = valid_frame_capture_fixture()
        fixture["captured_frames"][0]["mean_rgb"] = ["12", None, 30]

        with self.assertRaisesRegex(RuntimeError, "mean_rgb"):
            exemplar.validate_frame_capture_evidence(fixture)

    def test_frame_capture_fixture_rejects_offscreen_selected_window(self):
        fixture = valid_frame_capture_fixture()
        fixture["selected_window_visible_area"] = 0

        with self.assertRaisesRegex(RuntimeError, "offscreen player window"):
            exemplar.validate_frame_capture_evidence(fixture)

    def test_bridge_dry_run_can_validate_frame_capture_fixture_without_hud(self):
        with tempfile.TemporaryDirectory(dir=".") as tmpdir:
            output_dir = Path(tmpdir) / "bridge"
            fixture_path = output_dir / "frame-capture.json"
            output_dir.mkdir(parents=True)
            fixture_path.write_text(
                json.dumps(valid_frame_capture_fixture()),
                encoding="utf-8",
            )
            args = exemplar.build_parser().parse_args(
                [
                    "youtube-bridge",
                    "--dry-run",
                    "--media-ingress-dry-run",
                    "--frame-capture-fixture-json",
                    str(fixture_path),
                    "--output-dir",
                    str(output_dir),
                ]
            )
            evidence = asyncio.run(exemplar.run_youtube_bridge(args))

            self.assertFalse(evidence["media_ingress_open_attempted"])
            self.assertTrue(evidence["captured_youtube_frames_available_to_bridge"])
            self.assertTrue(evidence["frame_capture"]["capture_validated"])
            self.assertFalse(evidence["hud_runtime_receives_youtube_frames"])

    def test_frame_capture_fixture_accepts_windows_utf8_bom(self):
        with tempfile.TemporaryDirectory(dir=".") as tmpdir:
            fixture_path = Path(tmpdir) / "frame-capture.json"
            fixture_path.write_bytes(
                b"\xef\xbb\xbf"
                + json.dumps(valid_frame_capture_fixture()).encode("utf-8")
            )
            args = exemplar.build_parser().parse_args(
                [
                    "youtube-bridge",
                    "--dry-run",
                    "--media-ingress-dry-run",
                    "--frame-capture-fixture-json",
                    str(fixture_path),
                ]
            )

            evidence = exemplar.load_frame_capture_fixture(str(fixture_path), args)

            self.assertTrue(evidence["capture_validated"])

    def test_youtube_bridge_parser_defaults_to_bridge_agent(self):
        args = exemplar.build_parser().parse_args(
            ["youtube-bridge", "--dry-run", "--media-ingress-dry-run"]
        )

        self.assertEqual(args.command, "youtube-bridge")
        self.assertEqual(args.agent_id, exemplar.YOUTUBE_BRIDGE_AGENT_ID)
        self.assertEqual(args.source_label, exemplar.YOUTUBE_BRIDGE_SOURCE_LABEL)
        self.assertEqual(args.zone_name, exemplar.APPROVED_MEDIA_ZONE)
        self.assertEqual(args.capture_frame_samples, 3)

    def test_youtube_bridge_live_path_requires_windows_capture_adapter(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            args = exemplar.build_parser().parse_args(
                [
                    "youtube-bridge",
                    "--dry-run",
                    "--psk",
                    "test-psk",
                    "--output-dir",
                    tmpdir,
                ]
            )

            with self.assertRaisesRegex(RuntimeError, "Windows frame-capture adapter requires"):
                asyncio.run(exemplar.run_youtube_bridge(args))

    def test_youtube_bridge_live_path_requires_psk_before_frame_capture(self):
        args = exemplar.build_parser().parse_args(["youtube-bridge", "--dry-run"])

        with self.assertRaisesRegex(RuntimeError, "set TZE_HUD_PSK"):
            asyncio.run(exemplar.run_youtube_bridge(args))

    def test_invalid_approved_zone_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "approved zone"):
            exemplar.validate_approved_media_zone("pip")


if __name__ == "__main__":
    unittest.main()
