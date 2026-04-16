from __future__ import annotations

import sys
import unittest
import uuid
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

import presence_card_exemplar  # noqa: E402
from proto_gen import types_pb2  # noqa: E402


class PresenceCardExemplarTests(unittest.TestCase):
    def test_format_last_active_humanizes_elapsed_time(self) -> None:
        self.assertEqual(presence_card_exemplar.format_last_active(0), "now")
        self.assertEqual(presence_card_exemplar.format_last_active(29), "29s ago")
        self.assertEqual(presence_card_exemplar.format_last_active(30), "30s ago")
        self.assertEqual(presence_card_exemplar.format_last_active(60), "1m ago")
        self.assertEqual(presence_card_exemplar.format_last_active(125), "2m ago")

    def test_build_presence_card_mutations_include_tile_setup_and_three_nodes(self) -> None:
        tile_id = uuid.uuid4().bytes
        root_uuid = uuid.UUID("11111111-2222-7333-8444-555555555555")

        mutations = presence_card_exemplar.build_presence_card_mutations(
            tile_id=tile_id,
            agent_name="agent-alpha",
            avatar_rgba=(66 / 255.0, 133 / 255.0, 244 / 255.0, 1.0),
            elapsed_seconds=0,
            include_tile_setup=True,
            root_uuid=root_uuid,
        )

        self.assertEqual(len(mutations), 15)
        self.assertAlmostEqual(mutations[0].update_tile_opacity.opacity, 1.0)
        self.assertEqual(
            mutations[1].update_tile_input_mode.input_mode,
            types_pb2.TILE_INPUT_MODE_CAPTURE,
        )
        self.assertEqual(mutations[2].WhichOneof("mutation"), "set_tile_root")
        for mutation in mutations[3:]:
            self.assertEqual(mutation.WhichOneof("mutation"), "add_node")
            self.assertEqual(mutation.add_node.parent_id, root_uuid.bytes)
        self.assertEqual(
            mutations[7].add_node.node.text_markdown.content,
            "RESIDENT AGENT",
        )
        self.assertEqual(
            mutations[8].add_node.node.text_markdown.content,
            "**agent-alpha**",
        )
        self.assertEqual(
            mutations[9].add_node.node.text_markdown.content,
            "Connected • last active now",
        )
        self.assertEqual(
            mutations[11].add_node.node.text_markdown.content,
            "NOW",
        )
        self.assertEqual(
            mutations[13].add_node.node.text_markdown.content,
            "X",
        )
        self.assertEqual(
            mutations[14].add_node.node.hit_region.interaction_id,
            "dismiss-card",
        )

    def test_build_presence_card_mutations_for_update_rebuilds_full_tree(self) -> None:
        tile_id = uuid.uuid4().bytes
        root_uuid = uuid.UUID("aaaaaaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee")

        mutations = presence_card_exemplar.build_presence_card_mutations(
            tile_id=tile_id,
            agent_name="agent-gamma",
            avatar_rgba=(251 / 255.0, 188 / 255.0, 4 / 255.0, 1.0),
            elapsed_seconds=90,
            include_tile_setup=False,
            root_uuid=root_uuid,
        )

        self.assertEqual(len(mutations), 13)
        self.assertEqual(mutations[0].WhichOneof("mutation"), "set_tile_root")
        for mutation in mutations[1:]:
            self.assertEqual(mutation.WhichOneof("mutation"), "add_node")
        self.assertEqual(
            mutations[7].add_node.node.text_markdown.content,
            "Connected • last active 1m ago",
        )
        self.assertEqual(
            mutations[9].add_node.node.text_markdown.content,
            "1M",
        )
        self.assertEqual(
            mutations[11].add_node.node.text_markdown.content,
            "X",
        )
        self.assertEqual(
            mutations[12].add_node.node.hit_region.interaction_id,
            "dismiss-card",
        )

    def test_avatar_square_has_even_inset_within_plate(self) -> None:
        plate_center_x = (
            presence_card_exemplar.AVATAR_PLATE_X
            + presence_card_exemplar.AVATAR_PLATE_W / 2.0
        )
        plate_center_y = (
            presence_card_exemplar.AVATAR_PLATE_Y
            + presence_card_exemplar.AVATAR_PLATE_H / 2.0
        )
        avatar_center_x = (
            presence_card_exemplar.AVATAR_X
            + presence_card_exemplar.AVATAR_W / 2.0
        )
        avatar_center_y = (
            presence_card_exemplar.AVATAR_Y
            + presence_card_exemplar.AVATAR_H / 2.0
        )

        self.assertEqual(plate_center_x, avatar_center_x)
        self.assertEqual(plate_center_y, avatar_center_y)
        self.assertEqual(
            presence_card_exemplar.AVATAR_X - presence_card_exemplar.AVATAR_PLATE_X,
            presence_card_exemplar.AVATAR_PLATE_W
            - presence_card_exemplar.AVATAR_W
            - (presence_card_exemplar.AVATAR_X - presence_card_exemplar.AVATAR_PLATE_X),
        )
        self.assertEqual(
            presence_card_exemplar.AVATAR_Y - presence_card_exemplar.AVATAR_PLATE_Y,
            presence_card_exemplar.AVATAR_PLATE_H
            - presence_card_exemplar.AVATAR_H
            - (presence_card_exemplar.AVATAR_Y - presence_card_exemplar.AVATAR_PLATE_Y),
        )

    def test_build_step_plan_tracks_create_update_disconnect_cleanup(self) -> None:
        steps = presence_card_exemplar.build_step_plan(
            update_wait_s=30,
            heartbeat_timeout_s=15,
            orphan_grace_s=30,
            disconnect_agent_name="agent-gamma",
        )

        self.assertEqual([step["code"] for step in steps], [
            "create",
            "update_wait",
            "disconnect",
            "orphan_observe",
            "cleanup_wait",
            "final_state",
        ])
        self.assertIn("3 stacked cards", steps[0]["expected_visual"])
        self.assertIn("30s ago", steps[1]["expected_visual"])
        self.assertIn("agent-gamma", steps[2]["action"])
        self.assertIn("disconnection badge", steps[3]["expected_visual"])
        self.assertIn("2 remaining cards", steps[5]["expected_visual"])


if __name__ == "__main__":
    unittest.main()
