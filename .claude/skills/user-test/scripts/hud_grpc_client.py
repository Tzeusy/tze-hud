#!/usr/bin/env python3
"""
Python gRPC client for tze_hud session protocol.

Provides a high-level HudClient class for user-test scripts that need
to exercise tile creation, lease management, and node tree mutations
over the bidirectional gRPC session stream.

Usage:
    from hud_grpc_client import HudClient

    async with HudClient("tzehouse-windows.parrot-hen.ts.net:50051",
                          psk="tze-hud-key",
                          agent_id="test-agent") as client:
        lease_id = await client.request_lease(ttl_ms=60000)
        avatar_png = make_avatar_png((66, 133, 244))
        avatar_resource_id = await client.upload_avatar_png(avatar_png)
        tile_id = await client.create_presence_card_tile(
            lease_id,
            tab_id=None,
            agent_name="agent-alpha",
            avatar_resource_id=avatar_resource_id,
        )
        await client.session_close(expect_resume=False)
"""

from __future__ import annotations

import asyncio
import io
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from collections.abc import Callable
from functools import lru_cache
from pathlib import Path
from typing import Any, Optional

import grpc

# Proto stubs are in proto_gen/ relative to this file.
_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _SCRIPT_DIR)
sys.path.insert(0, os.path.join(_SCRIPT_DIR, "proto_gen"))

from proto_gen import session_pb2, session_pb2_grpc, types_pb2


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _now_wall_us() -> int:
    """Current UTC wall-clock in microseconds since epoch."""
    return int(time.time() * 1_000_000)


def _uuid_bytes() -> bytes:
    """Generate a 16-byte UUID for batch/request IDs."""
    return uuid.uuid4().bytes


def _png_image_size(png_bytes: bytes) -> tuple[int, int]:
    """Return the dimensions of a PNG payload."""
    from PIL import Image

    with Image.open(io.BytesIO(png_bytes)) as img:
        img.load()
        return img.width, img.height


def make_avatar_png(rgb: tuple[int, int, int]) -> bytes:
    """Build a solid-color 32x32 PNG avatar fixture."""
    from PIL import Image

    if len(rgb) != 3:
        raise ValueError("avatar rgb must be a 3-tuple")
    if any(channel < 0 or channel > 255 for channel in rgb):
        raise ValueError("avatar rgb values must be 0..255")

    image = Image.new("RGB", (32, 32), rgb)
    buf = io.BytesIO()
    image.save(buf, format="PNG")
    return buf.getvalue()


@lru_cache(maxsize=1)
def _blake3_helper_path() -> str:
    """Build a tiny cached cargo helper that prints a BLAKE3 digest in hex."""
    helper_override = os.getenv("HUD_GRPC_BLAKE3_HELPER")
    if helper_override:
        helper_path = Path(helper_override)
        if not helper_path.exists():
            raise RuntimeError(
                f"HUD_GRPC_BLAKE3_HELPER is set but file does not exist: {helper_path}"
            )
        return str(helper_path)

    cargo = shutil.which("cargo")
    if cargo is None:
        raise RuntimeError(
            "Could not compute BLAKE3 digest: install Python package 'blake3', "
            "or set HUD_GRPC_BLAKE3_HELPER to a prebuilt helper binary, "
            "or install cargo for on-demand helper compilation."
        )

    helper_dir = Path(tempfile.mkdtemp(prefix="hud-grpc-blake3-"))
    (helper_dir / "src").mkdir(parents=True, exist_ok=True)
    (helper_dir / "Cargo.toml").write_text(
        """[package]
name = "hud-grpc-blake3"
version = "0.1.0"
edition = "2021"

[dependencies]
blake3 = "1"
hex = "0.4"
""",
        encoding="utf-8",
    )
    (helper_dir / "src" / "main.rs").write_text(
        """use std::io::Read;

fn main() {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf).unwrap();
    let hash = blake3::hash(&buf);
    println!("{}", hex::encode(hash.as_bytes()));
}
""",
        encoding="utf-8",
    )

    subprocess.run(
        [cargo, "build", "--quiet", "--release"],
        check=True,
        cwd=helper_dir,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    binary_name = "hud-grpc-blake3.exe" if os.name == "nt" else "hud-grpc-blake3"
    binary_path = helper_dir / "target" / "release" / binary_name
    if not binary_path.exists():
        raise RuntimeError(f"missing BLAKE3 helper binary at {binary_path}")
    return str(binary_path)


def _blake3_digest_bytes(data: bytes) -> bytes:
    """Compute a BLAKE3 digest without adding a Python dependency."""
    try:
        import blake3  # type: ignore

        return blake3.blake3(data).digest()
    except ModuleNotFoundError:
        pass

    try:
        helper = _blake3_helper_path()
    except RuntimeError as exc:
        raise RuntimeError(
            f"{exc} (input size={len(data)} bytes)"
        ) from exc
    proc = subprocess.run(
        [helper],
        input=data,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return bytes.fromhex(proc.stdout.decode("utf-8").strip())


def avatar_resource_id_from_png(png_bytes: bytes) -> bytes:
    """Validate a 32x32 PNG avatar and return its content-addressed ResourceId."""
    if _png_image_size(png_bytes) != (32, 32):
        raise ValueError("avatar PNG must be exactly 32x32 pixels")
    return _blake3_digest_bytes(png_bytes)

def _resource_id_bytes(resource_id: Any) -> bytes:
    """Normalize a raw resource id or ResourceIdProto into 32 bytes."""
    if isinstance(resource_id, types_pb2.ResourceIdProto):
        raw = resource_id.bytes
        if len(raw) != 32:
            raise ValueError("resource id proto bytes must be 32 bytes")
        return raw
    if isinstance(resource_id, (bytes, bytearray)):
        raw = bytes(resource_id)
        if len(raw) != 32:
            raise ValueError("resource id must be 32 bytes")
        return raw
    raise TypeError(f"unsupported resource id type: {type(resource_id)!r}")


def _resource_error_code_name(error_code: int) -> str:
    """Render a stable enum name for resource-upload failures."""
    try:
        return session_pb2.ResourceErrorCode.Name(error_code)
    except ValueError:
        return f"RESOURCE_ERROR_{error_code}"


def build_presence_card_root_node(
    width: float = 320.0,
    height: float = 112.0,
) -> types_pb2.NodeProto:
    """Build the presence card background root node."""
    return _make_node(
        {
            "solid_color": {
                "r": 0.10,
                "g": 0.14,
                "b": 0.19,
                "a": 0.72,
                "radius": 12.0,
            },
            "bounds": [0, 0, width, height],
        }
    )


def build_presence_card_sheen_node(width: float = 320.0) -> types_pb2.NodeProto:
    """Build the top sheen used by the glass presence card."""
    return _make_node(
        {
            "solid_color": {
                "r": 0.92,
                "g": 0.96,
                "b": 1.0,
                "a": 0.16,
            },
            "bounds": [0, 0, width, 2],
        }
    )


def build_presence_card_accent_node(
    accent_rgba: tuple[float, float, float, float],
) -> types_pb2.NodeProto:
    """Build the left accent rail used by the glass presence card."""
    return _make_node(
        {
            "solid_color": {
                "r": accent_rgba[0],
                "g": accent_rgba[1],
                "b": accent_rgba[2],
                "a": 0.78,
            },
            "bounds": [0, 18, 4, 76],
        }
    )


def build_presence_card_avatar_plate_node(
    accent_rgba: tuple[float, float, float, float],
) -> types_pb2.NodeProto:
    """Build the translucent plate behind the avatar."""
    return _make_node(
        {
            "solid_color": {
                "r": accent_rgba[0],
                "g": accent_rgba[1],
                "b": accent_rgba[2],
                "a": 0.22,
            },
            "bounds": [24, 28, 56, 56],
        }
    )


def build_presence_card_avatar_node(resource_id: Any) -> types_pb2.NodeProto:
    """Build the avatar node used by Presence Card."""
    return _make_node(
        {
            "static_image": {
                "resource_id": _resource_id_bytes(resource_id),
                "width": 32,
                "height": 32,
                "decoded_bytes": 32 * 32 * 4,
                "fit_mode": types_pb2.IMAGE_FIT_MODE_COVER,
            },
            "bounds": [34, 38, 36, 36],
        }
    )


def build_presence_card_eyebrow_node() -> types_pb2.NodeProto:
    """Build the uppercase metadata label."""
    return _make_node(
        {
            "text_markdown": {
                "content": "RESIDENT AGENT",
                "font_size_px": 11.0,
                "color": [0.72, 0.80, 0.90, 0.82],
            },
            "bounds": [96, 18, 152, 12],
        }
    )


def build_presence_card_name_node(agent_name: str) -> types_pb2.NodeProto:
    """Build the bold display-name line."""
    return _make_node(
        {
            "text_markdown": {
                "content": f"**{agent_name}**",
                "font_size_px": 20.0,
                "color": [0.97, 0.99, 1.0, 1.0],
            },
            "bounds": [96, 34, 152, 26],
        }
    )


def build_presence_card_text_node(
    agent_name: str,
    last_active_label: str = "now",
) -> types_pb2.NodeProto:
    """Build the status line used by Presence Card."""
    del agent_name
    return _make_node(
        {
            "text_markdown": {
                "content": f"Connected • last active {last_active_label}",
                "font_size_px": 13.0,
                "color": [0.82, 0.88, 0.94, 0.92],
            },
            "bounds": [96, 68, 148, 18],
        }
    )


def build_presence_card_chip_bg_node() -> types_pb2.NodeProto:
    """Build the compact time-chip background."""
    return _make_node(
        {
            "solid_color": {
                "r": 0.86,
                "g": 0.92,
                "b": 1.0,
                "a": 0.12,
            },
            "bounds": [224, 20, 44, 22],
        }
    )


def _presence_card_chip_label(last_active_label: str) -> str:
    if last_active_label == "now":
        return "NOW"
    if last_active_label.endswith("s ago"):
        return f"{last_active_label[:-5]}S"
    if last_active_label.endswith("m ago"):
        return f"{last_active_label[:-5]}M"
    return last_active_label.upper()


def build_presence_card_chip_text_node(last_active_label: str = "now") -> types_pb2.NodeProto:
    """Build the compact time-chip label."""
    return _make_node(
        {
            "text_markdown": {
                "content": _presence_card_chip_label(last_active_label),
                "font_size_px": 10.0,
                "color": [0.96, 0.98, 1.0, 0.96],
            },
            "bounds": [224, 21, 44, 20],
        }
    )


def build_presence_card_dismiss_bg_node() -> types_pb2.NodeProto:
    """Build the compact dismiss button background."""
    return _make_node(
        {
            "solid_color": {
                "r": 0.94,
                "g": 0.97,
                "b": 1.0,
                "a": 0.14,
                "radius": 8.0,
            },
            "bounds": [280, 18, 24, 24],
        }
    )


def build_presence_card_dismiss_text_node() -> types_pb2.NodeProto:
    """Build the dismiss button label."""
    return _make_node(
        {
            "text_markdown": {
                "content": "X",
                "font_size_px": 12.0,
                "color": [0.97, 0.99, 1.0, 0.98],
            },
            "bounds": [280, 18, 24, 24],
        }
    )


def build_presence_card_dismiss_hit_region_node() -> types_pb2.NodeProto:
    """Build the dismiss button hit target."""
    return _make_node(
        {
            "hit_region": {
                "interaction_id": "dismiss-card",
                "accepts_focus": True,
                "accepts_pointer": True,
            },
            "bounds": [280, 18, 24, 24],
        }
    )


def build_presence_card_add_node_mutations(
    tile_id: bytes,
    resource_id: Any,
    agent_name: str,
    last_active_label: str = "now",
    accent_rgba: tuple[float, float, float, float] = (66 / 255.0, 133 / 255.0, 244 / 255.0, 1.0),
    card_width: float = 320.0,
    card_height: float = 112.0,
) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto], list[types_pb2.MutationProto]]:
    """Build the glass Presence Card tree and its AddNode mutations."""
    root = build_presence_card_root_node(width=card_width, height=card_height)
    child_nodes = [
        build_presence_card_sheen_node(width=card_width),
        build_presence_card_accent_node(accent_rgba),
        build_presence_card_avatar_plate_node(accent_rgba),
        build_presence_card_avatar_node(resource_id),
        build_presence_card_eyebrow_node(),
        build_presence_card_name_node(agent_name),
        build_presence_card_text_node(agent_name, last_active_label),
        build_presence_card_chip_bg_node(),
        build_presence_card_chip_text_node(last_active_label),
        build_presence_card_dismiss_bg_node(),
        build_presence_card_dismiss_text_node(),
        build_presence_card_dismiss_hit_region_node(),
    ]
    mutations = [
        types_pb2.MutationProto(
            set_tile_root=types_pb2.SetTileRootMutation(
                tile_id=tile_id,
                node=root,
            )
        )
    ]
    mutations.extend(
        types_pb2.MutationProto(
            add_node=types_pb2.AddNodeMutation(
                tile_id=tile_id,
                parent_id=root.id,
                node=node,
            )
        )
        for node in child_nodes
    )
    return root, child_nodes, mutations


def build_presence_card_tree_mutations(
    tile_id: bytes,
    resource_id: Any,
    agent_name: str,
    last_active_label: str = "now",
    accent_rgba: tuple[float, float, float, float] = (66 / 255.0, 133 / 255.0, 244 / 255.0, 1.0),
    card_width: float = 320.0,
    card_height: float = 112.0,
) -> tuple[types_pb2.NodeProto, list[types_pb2.NodeProto], list[types_pb2.MutationProto]]:
    """Alias for build_presence_card_add_node_mutations()."""
    return build_presence_card_add_node_mutations(
        tile_id=tile_id,
        resource_id=resource_id,
        agent_name=agent_name,
        last_active_label=last_active_label,
        accent_rgba=accent_rgba,
        card_width=card_width,
        card_height=card_height,
    )


def _make_node(data: dict) -> types_pb2.NodeProto:
    """Build a NodeProto from a dict spec.

    Supported types:
      {"solid_color": {"r": f, "g": f, "b": f, "a": f}, "bounds": [x,y,w,h]}
      {"text_markdown": {"content": str, "font_size_px": f, "color": [r,g,b,a]}, "bounds": [x,y,w,h]}
      {"hit_region": {"interaction_id": str, "accepts_focus": bool, "accepts_pointer": bool}, "bounds": [x,y,w,h]}

    Optional fields:
      {"id": bytes}  # explicit NodeProto.id (otherwise a random UUID is assigned)
    """
    node = types_pb2.NodeProto(id=data.get("id", _uuid_bytes()))

    bounds = data.get("bounds", [0, 0, 100, 100])
    rect = types_pb2.Rect(x=bounds[0], y=bounds[1], width=bounds[2], height=bounds[3])

    if "solid_color" in data:
        c = data["solid_color"]
        node.solid_color.CopyFrom(types_pb2.SolidColorNodeProto(
            color=types_pb2.Rgba(r=c["r"], g=c["g"], b=c["b"], a=c.get("a", 1.0)),
            bounds=rect,
            radius=c.get("radius", -1.0),
        ))
    elif "text_markdown" in data:
        t = data["text_markdown"]
        color = t.get("color", [1.0, 1.0, 1.0, 1.0])
        tm = types_pb2.TextMarkdownNodeProto(
            content=t["content"],
            bounds=rect,
            font_size_px=t.get("font_size_px", 14.0),
            color=types_pb2.Rgba(r=color[0], g=color[1], b=color[2], a=color[3]),
        )
        bg = t.get("background")
        if bg:
            tm.background.CopyFrom(types_pb2.Rgba(r=bg[0], g=bg[1], b=bg[2], a=bg[3]))
        for run in t.get("color_runs", []):
            run_color = run.get("color", color)
            tm.color_runs.append(types_pb2.TextColorRunProto(
                start_byte=run["start_byte"],
                end_byte=run["end_byte"],
                color=types_pb2.Rgba(
                    r=run_color[0],
                    g=run_color[1],
                    b=run_color[2],
                    a=run_color[3],
                ),
            ))
        node.text_markdown.CopyFrom(tm)
    elif "hit_region" in data:
        h = data["hit_region"]
        node.hit_region.CopyFrom(types_pb2.HitRegionNodeProto(
            bounds=rect,
            interaction_id=h.get("interaction_id", ""),
            accepts_focus=h.get("accepts_focus", False),
            accepts_pointer=h.get("accepts_pointer", False),
        ))
    elif "static_image" in data:
        s = data["static_image"]
        node.static_image.CopyFrom(types_pb2.StaticImageNodeProto(
            resource_id=s["resource_id"],
            width=s["width"],
            height=s["height"],
            decoded_bytes=s.get("decoded_bytes", 0),
            fit_mode=s.get("fit_mode", types_pb2.IMAGE_FIT_MODE_UNSPECIFIED),
            bounds=rect,
        ))
    else:
        raise ValueError(f"Unknown node type in: {data}")

    return node


# ---------------------------------------------------------------------------
# HudClient
# ---------------------------------------------------------------------------

class HudClient:
    """Async gRPC client for the tze_hud session protocol."""

    def __init__(
        self,
        target: str,
        psk: str,
        agent_id: str = "user-test-agent",
        capabilities: Optional[list[str]] = None,
        initial_subscriptions: Optional[list[str]] = None,
    ):
        self.target = target
        self.psk = psk
        self.agent_id = agent_id
        self.capabilities = capabilities or [
            "create_tiles",
            "modify_own_tiles",
            "access_input_events",
            "upload_resource",
        ]
        self.initial_subscriptions = initial_subscriptions or ["SCENE_TOPOLOGY"]
        self._channel: Optional[grpc.aio.Channel] = None
        self._stream = None
        self._seq = 0
        self._server_seq = 0
        self.session_id: Optional[bytes] = None
        self.namespace: Optional[str] = None
        self.heartbeat_interval_ms: Optional[int] = None
        self.granted_capabilities: list[str] = []
        self.scene_snapshot_json: Optional[str] = None
        self.scene_display_area: Optional[tuple[float, float]] = None
        self._response_queue: asyncio.Queue = asyncio.Queue()
        self._deferred_responses: list[Any] = []
        self._response_wait_lock = asyncio.Lock()
        self._event_queue: asyncio.Queue = asyncio.Queue()
        self._reader_task: Optional[asyncio.Task] = None
        self._send_queue: Optional[asyncio.Queue] = None
        self._transport_closed = False
        self._session_close_sent = False

    async def __aenter__(self):
        await self.connect()
        return self

    async def __aexit__(self, *exc):
        await self.close()

    def _next_seq(self) -> int:
        self._seq += 1
        return self._seq

    async def connect(self):
        """Open channel, start session stream, perform handshake."""
        self._transport_closed = False
        self._session_close_sent = False
        self._response_queue = asyncio.Queue()
        self._deferred_responses = []
        self._channel = grpc.aio.insecure_channel(self.target)
        stub = session_pb2_grpc.HudSessionStub(self._channel)

        # Build the outbound request iterator — we'll feed messages via a queue.
        self._send_queue = asyncio.Queue()
        self._stream = stub.Session(self._request_iterator())

        # Send SessionInit
        init_msg = session_pb2.ClientMessage(
            sequence=self._next_seq(),
            timestamp_wall_us=_now_wall_us(),
            session_init=session_pb2.SessionInit(
                agent_id=self.agent_id,
                agent_display_name=self.agent_id,
                auth_credential=session_pb2.AuthCredential(
                    pre_shared_key=session_pb2.PreSharedKeyCredential(key=self.psk),
                ),
                requested_capabilities=self.capabilities,
                initial_subscriptions=self.initial_subscriptions,
                agent_timestamp_wall_us=_now_wall_us(),
                min_protocol_version=1000,
                max_protocol_version=1000,
            ),
        )
        await self._send_queue.put(init_msg)

        # Start background reader
        self._reader_task = asyncio.create_task(self._read_loop())

        # Wait for SessionEstablished
        resp = await self._wait_for("session_established", timeout=5.0)
        est = resp.session_established
        self.session_id = est.session_id
        self.namespace = est.namespace
        self.heartbeat_interval_ms = est.heartbeat_interval_ms
        self.granted_capabilities = list(est.granted_capabilities)
        print(f"  [grpc] Session established: namespace={self.namespace}, "
              f"caps={self.granted_capabilities}", flush=True)

        snapshot_resp = await self._wait_for("scene_snapshot", timeout=5.0)
        self.scene_snapshot_json = snapshot_resp.scene_snapshot.snapshot_json
        self.scene_display_area = self._extract_scene_display_area(self.scene_snapshot_json)
        if self.scene_display_area is not None:
            w, h = self.scene_display_area
            print(f"  [grpc] Scene display area: {w:g}x{h:g}", flush=True)

    @staticmethod
    def _extract_scene_display_area(snapshot_json: str) -> Optional[tuple[float, float]]:
        try:
            snapshot = json.loads(snapshot_json)
        except json.JSONDecodeError:
            return None
        display_area = snapshot.get("display_area") or snapshot.get("displayArea")
        if not isinstance(display_area, dict):
            return None
        width = display_area.get("width")
        height = display_area.get("height")
        if not isinstance(width, (int, float)) or not isinstance(height, (int, float)):
            return None
        if width <= 0 or height <= 0:
            return None
        return float(width), float(height)

    async def _request_iterator(self):
        """Async generator that yields ClientMessages from the send queue."""
        while True:
            msg = await self._send_queue.get()
            if msg is None:
                return
            yield msg

    async def _read_loop(self):
        """Background task that reads ServerMessages and dispatches them."""
        try:
            async for msg in self._stream:
                self._server_seq = msg.sequence
                if msg.WhichOneof("payload") == "event_batch":
                    await self._event_queue.put(msg.event_batch)
                else:
                    await self._response_queue.put(msg)
        except grpc.aio.AioRpcError as e:
            if e.code() != grpc.StatusCode.CANCELLED:
                print(f"  [grpc] Stream error: {e}", flush=True)
        except Exception as e:
            print(f"  [grpc] Reader error: {e}", flush=True)

    async def _wait_for(self, payload_name: str, timeout: float = 10.0) -> Any:
        """Wait for a ServerMessage with the given payload type."""
        deadline = time.monotonic() + timeout
        async with self._response_wait_lock:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(f"Timed out waiting for {payload_name}")
                msg = self._pop_deferred_response(
                    lambda candidate: candidate.WhichOneof("payload")
                    in {payload_name, "session_error"}
                )
                if msg is None:
                    try:
                        msg = await asyncio.wait_for(
                            self._response_queue.get(), timeout=remaining
                        )
                    except asyncio.TimeoutError:
                        raise TimeoutError(f"Timed out waiting for {payload_name}")

                which = msg.WhichOneof("payload")
                if which == payload_name:
                    return msg
                if which == "session_error":
                    raise RuntimeError(
                        f"Session error: {msg.session_error.code} — "
                        f"{msg.session_error.message} "
                        f"(hint: {msg.session_error.hint})"
                    )
                self._deferred_responses.append(msg)

    async def wait_for(self, payload_name: str, timeout: float = 10.0) -> Any:
        """Public wrapper for waiting on a specific server payload."""
        return await self._wait_for(payload_name, timeout)

    async def _send(self, **payload_kwargs) -> int:
        """Send a ClientMessage with the given payload field and return sequence."""
        sequence = self._next_seq()
        msg = session_pb2.ClientMessage(
            sequence=sequence,
            timestamp_wall_us=_now_wall_us(),
            **payload_kwargs,
        )
        if self._send_queue is None:
            raise RuntimeError("client transport has not been initialized")
        await self._send_queue.put(msg)
        return sequence

    async def _shutdown_transport(self):
        """Cancel background tasks and close the gRPC channel."""
        if self._transport_closed:
            return
        self._transport_closed = True
        if self._send_queue is not None:
            await self._send_queue.put(None)
        if self._reader_task:
            self._reader_task.cancel()
            try:
                await self._reader_task
            except (asyncio.CancelledError, Exception):
                pass
        if self._channel:
            await self._channel.close()

    async def session_close(self, reason: str = "test complete", expect_resume: bool = False):
        """Request a graceful session close, but leave the transport open."""
        if self._session_close_sent or self._transport_closed:
            return
        await self._send(
            session_close=session_pb2.SessionClose(
                reason=reason,
                expect_resume=expect_resume,
            )
        )
        self._session_close_sent = True

    async def drop_connection(self):
        """Unconditionally close the underlying gRPC transport without SessionClose."""
        await self._shutdown_transport()

    async def disconnect(
        self,
        graceful: bool = True,
        reason: str = "test complete",
        expect_resume: bool = False,
    ):
        """Disconnect the session by graceful close or by dropping transport."""
        if graceful:
            await self.session_close(reason=reason, expect_resume=expect_resume)
            await self._shutdown_transport()
        else:
            await self.drop_connection()

    async def release_lease(self, lease_id: bytes):
        """Release a lease, removing all its tiles immediately."""
        await self._send(
            lease_release=session_pb2.LeaseRelease(lease_id=lease_id)
        )
        resp = await self._wait_for("lease_response", timeout=5.0)
        print(f"  [grpc] Lease released", flush=True)

    async def open_media_ingress(
        self,
        *,
        client_stream_id: bytes,
        agent_sdp_offer: bytes,
        zone_name: str = "media-pip",
        content_classification: str = "household",
        declared_peak_kbps: int = 2_000,
        codec_preference: Optional[list[int]] = None,
        expires_at_wall_us: int = 0,
        timeout: float = 10.0,
    ) -> session_pb2.MediaIngressOpenResult:
        """Open a video-only media ingress stream on the approved media zone."""
        if not hasattr(session_pb2, "MediaIngressOpen"):
            raise RuntimeError(
                "session_pb2 is missing MediaIngressOpen; regenerate user-test proto stubs"
            )
        if len(client_stream_id) != 16:
            raise ValueError("client_stream_id must be exactly 16 bytes")
        if not agent_sdp_offer:
            raise ValueError("agent_sdp_offer is required for the local producer path")
        codecs = codec_preference or [session_pb2.VIDEO_H264_BASELINE]
        await self._send(
            media_ingress_open=session_pb2.MediaIngressOpen(
                client_stream_id=client_stream_id,
                transport=session_pb2.TransportDescriptor(
                    mode=session_pb2.WEBRTC_STANDARD,
                    agent_sdp_offer=agent_sdp_offer,
                    relay_hint=session_pb2.DIRECT,
                ),
                zone_name=zone_name,
                codec_preference=codecs,
                has_audio_track=False,
                has_video_track=True,
                content_classification=content_classification,
                expires_at_wall_us=expires_at_wall_us,
                declared_peak_kbps=declared_peak_kbps,
            )
        )
        resp = await self._wait_for("media_ingress_open_result", timeout=timeout)
        result = resp.media_ingress_open_result
        if not result.admitted:
            raise RuntimeError(
                f"Media ingress rejected [{result.reject_code}]: {result.reject_reason}"
            )
        print(
            "  [grpc] Media ingress admitted: "
            f"epoch={result.stream_epoch} surface={result.assigned_surface_id.hex()} "
            f"codec={result.selected_codec}",
            flush=True,
        )
        return result

    async def close_media_ingress(
        self,
        stream_epoch: int,
        *,
        reason: str = "local producer complete",
        timeout: float = 10.0,
    ) -> session_pb2.MediaIngressCloseNotice:
        """Close an admitted media ingress stream and wait for the close notice."""
        await self._send(
            media_ingress_close=session_pb2.MediaIngressClose(
                stream_epoch=stream_epoch,
                reason=reason,
            )
        )
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Timed out waiting for media_ingress_close_notice")
            resp = await self._wait_for("media_ingress_close_notice", timeout=remaining)
            notice = resp.media_ingress_close_notice
            if notice.stream_epoch == stream_epoch:
                print(
                    f"  [grpc] Media ingress closed: epoch={stream_epoch} "
                    f"reason={notice.reason}",
                    flush=True,
                )
                return notice
            self._deferred_responses.append(resp)

    async def close(self, reason: str = "test complete", expect_resume: bool = False):
        """Gracefully close the session."""
        try:
            if not self._session_close_sent and not self._transport_closed:
                await self.session_close(reason=reason, expect_resume=expect_resume)
        except Exception:
            pass
        await self._shutdown_transport()

    # ─── Lease management ─────────────────────────────────────────────────

    async def request_lease(
        self, ttl_ms: int = 60000, priority: int = 2
    ) -> bytes:
        """Request a lease and return the granted lease_id."""
        await self._send(
            lease_request=session_pb2.LeaseRequest(
                ttl_ms=ttl_ms,
                capabilities=self.capabilities,
                lease_priority=priority,
            )
        )
        resp = await self._wait_for("lease_response", timeout=5.0)
        lr = resp.lease_response
        if not lr.granted:
            deny_reason = getattr(lr, "deny_reason", "") or "unspecified denial"
            deny_code = getattr(lr, "deny_code", "")
            if deny_code:
                raise RuntimeError(f"Lease denied [{deny_code}]: {deny_reason}")
            raise RuntimeError(f"Lease denied: {deny_reason}")
        print(f"  [grpc] Lease granted: ttl={lr.granted_ttl_ms}ms, "
              f"priority={lr.granted_priority}", flush=True)
        return lr.lease_id

    async def _await_resource_upload_result(
        self,
        request_sequence: int,
        timeout: float = 10.0,
    ) -> session_pb2.ResourceStored:
        """Wait for ResourceStored/ResourceErrorResponse correlated to a request sequence."""
        deadline = time.monotonic() + timeout
        async with self._response_wait_lock:
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(
                        f"Timed out waiting for resource upload result "
                        f"(request_sequence={request_sequence})"
                    )
                msg = self._pop_deferred_response(
                    lambda candidate: self._matches_resource_upload_wait(
                        candidate, request_sequence
                    )
                )
                if msg is None:
                    try:
                        msg = await asyncio.wait_for(
                            self._response_queue.get(), timeout=remaining
                        )
                    except asyncio.TimeoutError:
                        raise TimeoutError(
                            f"Timed out waiting for resource upload result "
                            f"(request_sequence={request_sequence})"
                        )

                if not self._matches_resource_upload_wait(msg, request_sequence):
                    self._deferred_responses.append(msg)
                    continue

                which = msg.WhichOneof("payload")
                if which == "resource_stored":
                    return msg.resource_stored
                if which == "resource_error_response":
                    err = msg.resource_error_response
                    code_name = _resource_error_code_name(err.error_code)
                    raise RuntimeError(
                        f"Resource upload failed [{code_name}]: {err.message} "
                        f"(context: {err.context}, hint: {err.hint})"
                    )
                if which == "session_error":
                    raise RuntimeError(
                        f"Session error while waiting for upload result: "
                        f"{msg.session_error.code} — {msg.session_error.message} "
                        f"(hint: {msg.session_error.hint})"
                    )
                if which == "runtime_error":
                    raise RuntimeError(
                        f"Runtime error while waiting for upload result: "
                        f"{msg.runtime_error.error_code} — {msg.runtime_error.message}"
                    )

    def _pop_deferred_response(
        self,
        matcher: Callable[[Any], bool],
    ) -> Any | None:
        """Return the first deferred response matching matcher, if any."""
        for index, msg in enumerate(self._deferred_responses):
            if matcher(msg):
                return self._deferred_responses.pop(index)
        return None

    @staticmethod
    def _matches_resource_upload_wait(msg: Any, request_sequence: int) -> bool:
        which = msg.WhichOneof("payload")
        if which == "resource_stored":
            return msg.resource_stored.request_sequence == request_sequence
        if which == "resource_error_response":
            return msg.resource_error_response.request_sequence == request_sequence
        return which in {"session_error", "runtime_error"}

    async def upload_png_resource(
        self,
        png_bytes: bytes,
        *,
        timeout: float = 10.0,
    ) -> bytes:
        """Upload a PNG via resident ResourceUploadStart and return ResourceId bytes."""
        if len(png_bytes) > 64 * 1024:
            raise ValueError(
                "PNG exceeds 64 KiB inline upload limit; chunked upload is not implemented"
            )
        width, height = _png_image_size(png_bytes)
        request_sequence = await self._send(
            resource_upload_start=session_pb2.ResourceUploadStart(
                expected_hash=_blake3_digest_bytes(png_bytes),
                resource_type=session_pb2.IMAGE_PNG,
                total_size_bytes=len(png_bytes),
                metadata=session_pb2.ResourceMetadata(width=width, height=height),
                inline_data=png_bytes,
            )
        )
        stored = await self._await_resource_upload_result(
            request_sequence=request_sequence,
            timeout=timeout,
        )
        resource_id = _resource_id_bytes(stored.resource_id)
        print(
            f"  [grpc] Resource uploaded: {resource_id.hex()[:16]}... "
            f"bytes={len(png_bytes)} dedup={stored.was_deduplicated}",
            flush=True,
        )
        return resource_id

    async def upload_avatar_png(
        self,
        png_bytes: bytes,
        *,
        timeout: float = 10.0,
    ) -> bytes:
        """Upload a 32x32 PNG avatar via resident upload flow and return ResourceId."""
        if _png_image_size(png_bytes) != (32, 32):
            raise ValueError("avatar PNG must be exactly 32x32 pixels")
        return await self.upload_png_resource(png_bytes, timeout=timeout)

    async def apply_mutations(
        self,
        lease_id: bytes,
        mutations: list[types_pb2.MutationProto],
    ) -> session_pb2.MutationResult:
        """Submit a raw mutation batch and return the acknowledged result."""
        batch_id = _uuid_bytes()
        await self._send(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=mutations,
            )
        )
        resp = await self._wait_for("mutation_result", timeout=5.0)
        mr = resp.mutation_result
        if not mr.accepted:
            raise RuntimeError(
                f"Mutation batch rejected: {mr.error_code} — {mr.error_message}"
            )
        return mr

    # ─── Tile operations ──────────────────────────────────────────────────

    async def submit_mutation_batch(
        self,
        lease_id: bytes,
        mutations: list[types_pb2.MutationProto],
        timeout: float = 5.0,
    ) -> session_pb2.MutationResult:
        """Submit a mutation batch and return the accepted result."""
        batch_id = _uuid_bytes()
        await self._send(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=mutations,
            )
        )
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(
                    "Timed out waiting for mutation_result for submitted batch"
                )
            resp = await self._wait_for("mutation_result", timeout=remaining)
            mr = resp.mutation_result
            if mr.batch_id != batch_id:
                continue
            if not mr.accepted:
                raise RuntimeError(
                    f"Mutation batch rejected: {mr.error_code} — {mr.error_message}"
                )
            return mr

    async def create_tile(
        self,
        lease_id: bytes,
        tab_id: Optional[bytes] = None,
        x: float = 50,
        y: float = 50,
        w: float = 400,
        h: float = 300,
        z_order: int = 100,
    ) -> bytes:
        """Create a tile and return its SceneId."""
        mr = await self.submit_mutation_batch(
            lease_id,
            [
                types_pb2.MutationProto(
                    create_tile=types_pb2.CreateTileMutation(
                        tab_id=tab_id or b"",
                        bounds=types_pb2.Rect(x=x, y=y, width=w, height=h),
                        z_order=z_order,
                    )
                )
            ],
        )
        tile_id = mr.created_ids[0]
        print(f"  [grpc] Tile created: {tile_id.hex()[:16]}...", flush=True)
        return tile_id

    async def update_tile_opacity(
        self,
        lease_id: bytes,
        tile_id: bytes,
        opacity: float,
    ) -> None:
        """Update a tile's opacity."""
        await self.submit_mutation_batch(
            lease_id,
            [
                types_pb2.MutationProto(
                    update_tile_opacity=types_pb2.UpdateTileOpacityMutation(
                        tile_id=tile_id,
                        opacity=opacity,
                    )
                )
            ],
        )
        print(f"  [grpc] Tile opacity set to {opacity}", flush=True)

    async def update_tile_input_mode(
        self,
        lease_id: bytes,
        tile_id: bytes,
        input_mode: types_pb2.TileInputModeProto,
    ) -> None:
        """Update a tile's input mode."""
        await self.submit_mutation_batch(
            lease_id,
            [
                types_pb2.MutationProto(
                    update_tile_input_mode=types_pb2.UpdateTileInputModeMutation(
                        tile_id=tile_id,
                        input_mode=input_mode,
                    )
                )
            ],
        )
        print("  [grpc] Tile input mode set", flush=True)

    async def set_tile_root(
        self,
        lease_id: bytes,
        tile_id: bytes,
        node_spec: Any,
    ):
        """Set a tile's root node from a dict spec or NodeProto."""
        node = node_spec if isinstance(node_spec, types_pb2.NodeProto) else _make_node(node_spec)
        await self.submit_mutation_batch(
            lease_id,
            [
                types_pb2.MutationProto(
                    set_tile_root=types_pb2.SetTileRootMutation(
                        tile_id=tile_id,
                        node=node,
                    )
                )
            ],
        )
        print(f"  [grpc] Tile root set", flush=True)

    async def add_node(
        self,
        lease_id: bytes,
        tile_id: bytes,
        node_spec: Any,
        parent_id: Optional[bytes] = None,
    ) -> bytes:
        """Add a child node to a tile and return the created node id."""
        node = node_spec if isinstance(node_spec, types_pb2.NodeProto) else _make_node(node_spec)
        mr = await self.submit_mutation_batch(
            lease_id,
            [
                types_pb2.MutationProto(
                    add_node=types_pb2.AddNodeMutation(
                        tile_id=tile_id,
                        parent_id=parent_id or b"",
                        node=node,
                    )
                )
            ],
        )
        node_id = mr.created_ids[0]
        print(f"  [grpc] Node added: {node_id.hex()[:16]}...", flush=True)
        return node_id

    async def update_node_content(
        self,
        lease_id: bytes,
        tile_id: bytes,
        node_id: bytes,
        node_spec: Any,
    ) -> None:
        """Replace a node's content in place."""
        node = node_spec if isinstance(node_spec, types_pb2.NodeProto) else _make_node(node_spec)
        mutation = types_pb2.UpdateNodeContentMutation(tile_id=tile_id, node_id=node_id)
        if node.HasField("solid_color"):
            mutation.solid_color.CopyFrom(node.solid_color)
        elif node.HasField("text_markdown"):
            mutation.text_markdown.CopyFrom(node.text_markdown)
        elif node.HasField("hit_region"):
            mutation.hit_region.CopyFrom(node.hit_region)
        elif node.HasField("static_image"):
            mutation.static_image.CopyFrom(node.static_image)
        await self.submit_mutation_batch(
            lease_id,
            [types_pb2.MutationProto(update_node_content=mutation)],
        )

    async def create_presence_card_tile(
        self,
        lease_id: bytes,
        tab_id: Optional[bytes],
        agent_name: str,
        avatar_resource_id: Any,
        *,
        accent_rgba: tuple[float, float, float, float] = (66 / 255.0, 133 / 255.0, 244 / 255.0, 1.0),
        x: float = 24.0,
        y: float = 0.0,
        w: float = 320.0,
        h: float = 112.0,
        z_order: int = 100,
    ) -> bytes:
        """Create a full Presence Card tile and return the tile id."""
        tile_id = await self.create_tile(
            lease_id,
            tab_id=tab_id,
            x=x,
            y=y,
            w=w,
            h=h,
            z_order=z_order,
        )
        await self.update_tile_opacity(lease_id, tile_id, 1.0)
        await self.update_tile_input_mode(
            lease_id,
            tile_id,
            types_pb2.TILE_INPUT_MODE_CAPTURE,
        )
        root, children, _ = build_presence_card_add_node_mutations(
            tile_id=tile_id,
            resource_id=avatar_resource_id,
            agent_name=agent_name,
            accent_rgba=accent_rgba,
            card_width=w,
            card_height=h,
        )
        await self.set_tile_root(lease_id, tile_id, root)
        for node in children:
            await self.add_node(lease_id, tile_id, node, parent_id=root.id)
        return tile_id

    async def send_heartbeat(self):
        """Send a keepalive heartbeat."""
        await self._send(heartbeat=session_pb2.Heartbeat())

    async def wait_for_click(
        self,
        interaction_id: str,
        timeout: Optional[float] = None,
    ):
        """Wait until an INPUT_EVENTS batch carries a matching ClickEvent."""
        while True:
            if timeout is None:
                batch = await self._event_queue.get()
            else:
                batch = await asyncio.wait_for(self._event_queue.get(), timeout=timeout)
            for envelope in batch.events:
                if envelope.WhichOneof("event") != "click":
                    continue
                if envelope.click.interaction_id == interaction_id:
                    return envelope.click


# ---------------------------------------------------------------------------
# Quick self-test
# ---------------------------------------------------------------------------

async def _self_test():
    """Connect, create a Presence Card tile, hold for 5s, close."""
    import argparse
    parser = argparse.ArgumentParser(description="gRPC client self-test")
    parser.add_argument("--target", default="tzehouse-windows.parrot-hen.ts.net:50051")
    parser.add_argument("--psk", default=os.getenv("MCP_TEST_PSK", "tze-hud-key"))
    args = parser.parse_args()

    print(f"Connecting to {args.target}...", flush=True)
    async with HudClient(args.target, psk=args.psk, agent_id="grpc-self-test") as client:
        lease_id = await client.request_lease(ttl_ms=30000)
        avatar_png = make_avatar_png((255, 0, 0))
        avatar_resource_id = await client.upload_avatar_png(avatar_png)
        tile_id = await client.create_presence_card_tile(
            lease_id,
            tab_id=None,
            agent_name="grpc-self-test",
            avatar_resource_id=avatar_resource_id,
            x=500,
            y=300,
            w=320,
            h=112,
            z_order=100,
        )
        print(
            f"  Presence card tile {tile_id.hex()[:16]}... visible for 10 seconds...",
            flush=True,
        )
        await asyncio.sleep(10)
        await client.release_lease(lease_id)
        print("  Closing session.", flush=True)


if __name__ == "__main__":
    asyncio.run(_self_test())
