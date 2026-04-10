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
    build_presence_card_root_node,
    build_presence_card_avatar_node,
    build_presence_card_text_node,
    make_avatar_png,
)
from proto_gen import types_pb2


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
        avatar = build_presence_card_avatar_node(resource_id)
        text = build_presence_card_text_node("agent-alpha")

        self.assertTrue(root.HasField("solid_color"))
        self.assertAlmostEqual(root.solid_color.color.r, 0.08, places=5)
        self.assertAlmostEqual(root.solid_color.color.a, 0.78, places=5)

        self.assertTrue(avatar.HasField("static_image"))
        self.assertEqual(avatar.static_image.resource_id, resource_id)
        self.assertEqual(avatar.static_image.width, 32)
        self.assertEqual(avatar.static_image.height, 32)
        self.assertEqual(
            avatar.static_image.fit_mode,
            types_pb2.IMAGE_FIT_MODE_COVER,
        )

        self.assertTrue(text.HasField("text_markdown"))
        self.assertIn("**agent-alpha**", text.text_markdown.content)
        self.assertIn("Last active: now", text.text_markdown.content)
        self.assertEqual(text.text_markdown.font_size_px, 14.0)

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
        client.add_node = AsyncMock(side_effect=[b"avatar-node", b"text-node"])

        lease_id = b"\x22" * 16
        tab_id = b"\x33" * 16
        avatar_resource_id = b"\x44" * 32

        tile_id = await client.create_presence_card_tile(
            lease_id,
            tab_id=tab_id,
            agent_name="agent-alpha",
            avatar_resource_id=avatar_resource_id,
            x=16.0,
            y=44.0,
            w=240.0,
            h=96.0,
            z_order=100,
        )

        self.assertEqual(tile_id, b"tile-id")
        client.create_tile.assert_awaited_once_with(
            lease_id,
            tab_id=tab_id,
            x=16.0,
            y=44.0,
            w=240.0,
            h=96.0,
            z_order=100,
        )
        client.update_tile_opacity.assert_awaited_once_with(lease_id, b"tile-id", 1.0)
        client.update_tile_input_mode.assert_awaited_once_with(
            lease_id,
            b"tile-id",
            types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
        )
        client.set_tile_root.assert_awaited_once()
        client.add_node.assert_awaited()
        root_node = client.set_tile_root.await_args.args[2]
        self.assertTrue(root_node.HasField("solid_color"))
        self.assertEqual(root_node.solid_color.bounds.width, 240.0)
        self.assertEqual(root_node.solid_color.bounds.height, 96.0)

        first_add = client.add_node.await_args_list[0]
        second_add = client.add_node.await_args_list[1]
        self.assertEqual(first_add.kwargs["parent_id"], root_node.id)
        self.assertEqual(second_add.kwargs["parent_id"], root_node.id)
        self.assertTrue(first_add.args[2].HasField("static_image"))
        self.assertTrue(second_add.args[2].HasField("text_markdown"))

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


if __name__ == "__main__":
    unittest.main()
