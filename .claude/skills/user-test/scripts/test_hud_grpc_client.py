import io
import unittest
from unittest.mock import AsyncMock

try:
    from PIL import Image
except ModuleNotFoundError:  # pragma: no cover - environment dependent
    Image = None

from hud_grpc_client import (
    HudClient,
    _resource_id_bytes,
    avatar_resource_id_from_png,
    build_presence_card_accent_node,
    build_presence_card_avatar_plate_node,
    build_presence_card_root_node,
    build_presence_card_avatar_node,
    build_presence_card_chip_bg_node,
    build_presence_card_chip_text_node,
    build_presence_card_dismiss_bg_node,
    build_presence_card_dismiss_hit_region_node,
    build_presence_card_dismiss_text_node,
    build_presence_card_eyebrow_node,
    build_presence_card_name_node,
    build_presence_card_sheen_node,
    build_presence_card_text_node,
    make_avatar_png,
)
from proto_gen import session_pb2, types_pb2


class HudGrpcClientTests(unittest.IsolatedAsyncioTestCase):
    @unittest.skipIf(Image is None, "Pillow is required for PNG avatar tests")
    def test_make_avatar_png_is_32_by_32_png(self):
        png = make_avatar_png((66, 133, 244))
        with Image.open(io.BytesIO(png)) as img:
            self.assertEqual(img.size, (32, 32))
            self.assertEqual(img.format, "PNG")

    @unittest.skipIf(Image is None, "Pillow is required for PNG avatar tests")
    def test_avatar_resource_id_is_32_bytes_and_deterministic(self):
        png = make_avatar_png((52, 168, 83))
        rid1 = avatar_resource_id_from_png(png)
        rid2 = avatar_resource_id_from_png(png)
        self.assertEqual(len(rid1), 32)
        self.assertEqual(rid1, rid2)

    def test_presence_card_node_builders_match_spec(self):
        resource_id = b"\x11" * 32
        root = build_presence_card_root_node()
        sheen = build_presence_card_sheen_node()
        accent = build_presence_card_accent_node((66 / 255.0, 133 / 255.0, 244 / 255.0, 1.0))
        plate = build_presence_card_avatar_plate_node((66 / 255.0, 133 / 255.0, 244 / 255.0, 1.0))
        avatar = build_presence_card_avatar_node(resource_id)
        eyebrow = build_presence_card_eyebrow_node()
        name = build_presence_card_name_node("agent-alpha")
        text = build_presence_card_text_node("agent-alpha")
        chip_bg = build_presence_card_chip_bg_node()
        chip_text = build_presence_card_chip_text_node("now")
        dismiss_bg = build_presence_card_dismiss_bg_node()
        dismiss_text = build_presence_card_dismiss_text_node()
        dismiss_hit_region = build_presence_card_dismiss_hit_region_node()

        self.assertTrue(root.HasField("solid_color"))
        self.assertAlmostEqual(root.solid_color.color.r, 0.10, places=5)
        self.assertAlmostEqual(root.solid_color.color.a, 0.72, places=5)
        self.assertEqual(root.solid_color.bounds.width, 320.0)
        self.assertEqual(root.solid_color.bounds.height, 112.0)

        self.assertTrue(sheen.HasField("solid_color"))
        self.assertEqual(sheen.solid_color.bounds.height, 2.0)
        self.assertTrue(accent.HasField("solid_color"))
        self.assertEqual(accent.solid_color.bounds.width, 4.0)
        self.assertTrue(plate.HasField("solid_color"))
        self.assertEqual(plate.solid_color.bounds.width, 56.0)
        self.assertEqual(plate.solid_color.bounds.height, 56.0)

        self.assertTrue(avatar.HasField("static_image"))
        self.assertEqual(avatar.static_image.resource_id, resource_id)
        self.assertEqual(avatar.static_image.width, 32)
        self.assertEqual(avatar.static_image.height, 32)
        self.assertEqual(
            avatar.static_image.fit_mode,
            types_pb2.IMAGE_FIT_MODE_COVER,
        )
        self.assertEqual(avatar.static_image.bounds.x, 34.0)
        self.assertEqual(avatar.static_image.bounds.width, 36.0)

        self.assertTrue(eyebrow.HasField("text_markdown"))
        self.assertEqual(eyebrow.text_markdown.content, "RESIDENT AGENT")
        self.assertEqual(eyebrow.text_markdown.font_size_px, 11.0)

        self.assertTrue(name.HasField("text_markdown"))
        self.assertEqual(name.text_markdown.content, "**agent-alpha**")
        self.assertEqual(name.text_markdown.font_size_px, 20.0)

        self.assertTrue(text.HasField("text_markdown"))
        self.assertEqual(text.text_markdown.content, "Connected • last active now")
        self.assertEqual(text.text_markdown.font_size_px, 13.0)

        self.assertTrue(chip_bg.HasField("solid_color"))
        self.assertEqual(chip_bg.solid_color.bounds.width, 44.0)

        self.assertTrue(chip_text.HasField("text_markdown"))
        self.assertEqual(chip_text.text_markdown.content, "NOW")
        self.assertEqual(chip_text.text_markdown.font_size_px, 10.0)

        self.assertTrue(dismiss_bg.HasField("solid_color"))
        self.assertEqual(dismiss_bg.solid_color.bounds.width, 24.0)
        self.assertEqual(dismiss_bg.solid_color.bounds.height, 24.0)

        self.assertTrue(dismiss_text.HasField("text_markdown"))
        self.assertEqual(dismiss_text.text_markdown.content, "X")

        self.assertTrue(dismiss_hit_region.HasField("hit_region"))
        self.assertEqual(dismiss_hit_region.hit_region.interaction_id, "dismiss-card")
        self.assertTrue(dismiss_hit_region.hit_region.accepts_pointer)

    def test_resource_id_bytes_rejects_invalid_proto_length(self):
        rid = types_pb2.ResourceIdProto(bytes=b"\x01" * 31)
        with self.assertRaises(ValueError):
            _resource_id_bytes(rid)

    async def test_create_presence_card_tile_sequences_helper_calls(self):
        client = HudClient("example.invalid:50051", psk="test-key")
        client.create_tile = AsyncMock(return_value=b"tile-id")
        client.update_tile_opacity = AsyncMock()
        client.update_tile_input_mode = AsyncMock()
        client.set_tile_root = AsyncMock()
        client.add_node = AsyncMock(side_effect=[f"node-{idx}".encode() for idx in range(12)])

        lease_id = b"\x22" * 16
        tab_id = b"\x33" * 16
        avatar_resource_id = b"\x44" * 32

        tile_id = await client.create_presence_card_tile(
            lease_id,
            tab_id=tab_id,
            agent_name="agent-alpha",
            avatar_resource_id=avatar_resource_id,
            x=24.0,
            y=44.0,
            w=320.0,
            h=112.0,
            z_order=100,
        )

        self.assertEqual(tile_id, b"tile-id")
        client.create_tile.assert_awaited_once_with(
            lease_id,
            tab_id=tab_id,
            x=24.0,
            y=44.0,
            w=320.0,
            h=112.0,
            z_order=100,
        )
        client.update_tile_opacity.assert_awaited_once_with(lease_id, b"tile-id", 1.0)
        client.update_tile_input_mode.assert_awaited_once_with(
            lease_id,
            b"tile-id",
            types_pb2.TILE_INPUT_MODE_CAPTURE,
        )
        client.set_tile_root.assert_awaited_once()
        client.add_node.assert_awaited()
        root_node = client.set_tile_root.await_args.args[2]
        self.assertTrue(root_node.HasField("solid_color"))
        self.assertEqual(root_node.solid_color.bounds.width, 320.0)
        self.assertEqual(root_node.solid_color.bounds.height, 112.0)

        self.assertEqual(client.add_node.await_count, 12)
        for awaited in client.add_node.await_args_list:
            self.assertEqual(awaited.kwargs["parent_id"], root_node.id)

        self.assertTrue(client.add_node.await_args_list[0].args[2].HasField("solid_color"))
        self.assertTrue(client.add_node.await_args_list[3].args[2].HasField("static_image"))
        self.assertEqual(
            client.add_node.await_args_list[4].args[2].text_markdown.content,
            "RESIDENT AGENT",
        )
        self.assertEqual(
            client.add_node.await_args_list[5].args[2].text_markdown.content,
            "**agent-alpha**",
        )
        self.assertEqual(
            client.add_node.await_args_list[6].args[2].text_markdown.content,
            "Connected • last active now",
        )
        self.assertEqual(
            client.add_node.await_args_list[8].args[2].text_markdown.content,
            "NOW",
        )
        self.assertTrue(client.add_node.await_args_list[9].args[2].HasField("solid_color"))
        self.assertEqual(
            client.add_node.await_args_list[10].args[2].text_markdown.content,
            "X",
        )
        self.assertEqual(
            client.add_node.await_args_list[11].args[2].hit_region.interaction_id,
            "dismiss-card",
        )

    async def test_disconnect_primitives_split_graceful_and_hard_paths(self):
        client = HudClient("example.invalid:50051", psk="test-key")
        client._send = AsyncMock()
        client._shutdown_transport = AsyncMock()

        await client.session_close(expect_resume=False)
        client._send.assert_awaited_once()
        close_kwargs = client._send.await_args.kwargs
        self.assertIn("session_close", close_kwargs)
        self.assertFalse(close_kwargs["session_close"].expect_resume)

        client._send.reset_mock()
        client._shutdown_transport.reset_mock()
        client._session_close_sent = False

        await client.disconnect(graceful=True)
        client._send.assert_awaited_once()
        client._shutdown_transport.assert_awaited_once()

        client._send.reset_mock()
        client._shutdown_transport.reset_mock()

        await client.disconnect(graceful=False)
        client._shutdown_transport.assert_awaited_once()
        client._send.assert_not_called()

    async def test_request_lease_reports_proto_deny_reason(self):
        client = HudClient("example.invalid:50051", psk="test-key")
        client._send = AsyncMock()
        client._wait_for = AsyncMock(
            return_value=session_pb2.ServerMessage(
                lease_response=session_pb2.LeaseResponse(
                    granted=False,
                    deny_reason="requested lease scope exceeds session-granted capabilities",
                    deny_code="PERMISSION_DENIED",
                )
            )
        )

        with self.assertRaisesRegex(
            RuntimeError,
            "PERMISSION_DENIED.*requested lease scope exceeds session-granted capabilities",
        ):
            await client.request_lease(ttl_ms=120_000)


if __name__ == "__main__":
    unittest.main()
