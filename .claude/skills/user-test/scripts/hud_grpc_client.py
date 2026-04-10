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
        avatar_resource_id = upload_avatar_png(avatar_png)
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
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from functools import lru_cache
from pathlib import Path
from typing import Any, Optional

import grpc

# Proto stubs are in proto_gen/ relative to this file.
_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, _SCRIPT_DIR)

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
    cargo = shutil.which("cargo")
    if cargo is None:
        raise RuntimeError("cargo is required to compute BLAKE3 digests here")

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

    helper = _blake3_helper_path()
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


def upload_avatar_png(png_bytes: bytes) -> bytes:
    """Alias for avatar_resource_id_from_png() for resident-flow scripts."""
    return avatar_resource_id_from_png(png_bytes)


def _resource_id_bytes(resource_id: Any) -> bytes:
    """Normalize a raw resource id or ResourceIdProto into 32 bytes."""
    if isinstance(resource_id, types_pb2.ResourceIdProto):
        return resource_id.bytes
    if isinstance(resource_id, (bytes, bytearray)):
        raw = bytes(resource_id)
        if len(raw) != 32:
            raise ValueError("resource id must be 32 bytes")
        return raw
    raise TypeError(f"unsupported resource id type: {type(resource_id)!r}")


def build_presence_card_root_node() -> types_pb2.NodeProto:
    """Build the presence card background root node."""
    return _make_node(
        {
            "solid_color": {
                "r": 0.08,
                "g": 0.08,
                "b": 0.08,
                "a": 0.78,
            },
            "bounds": [0, 0, 200, 80],
        }
    )


def build_presence_card_avatar_node(resource_id: Any) -> types_pb2.NodeProto:
    """Build the 32x32 avatar node used by Presence Card."""
    return _make_node(
        {
            "static_image": {
                "resource_id": _resource_id_bytes(resource_id),
                "width": 32,
                "height": 32,
                "decoded_bytes": 32 * 32 * 4,
                "fit_mode": types_pb2.IMAGE_FIT_MODE_COVER,
            },
            "bounds": [8, 24, 32, 32],
        }
    )


def build_presence_card_text_node(
    agent_name: str,
    last_active_label: str = "now",
) -> types_pb2.NodeProto:
    """Build the agent label/status text node used by Presence Card."""
    return _make_node(
        {
            "text_markdown": {
                "content": f"**{agent_name}**\nLast active: {last_active_label}",
                "font_size_px": 14.0,
                "color": [0.94, 0.94, 0.94, 1.0],
            },
            "bounds": [48, 8, 144, 64],
        }
    )


def build_presence_card_add_node_mutations(
    tile_id: bytes,
    resource_id: Any,
    agent_name: str,
    last_active_label: str = "now",
) -> tuple[types_pb2.NodeProto, types_pb2.NodeProto, types_pb2.NodeProto, list[types_pb2.MutationProto]]:
    """Build the 3-node Presence Card tree and its AddNode mutations."""
    root = build_presence_card_root_node()
    avatar = build_presence_card_avatar_node(resource_id)
    text = build_presence_card_text_node(agent_name, last_active_label)
    mutations = [
        types_pb2.MutationProto(
            set_tile_root=types_pb2.SetTileRootMutation(
                tile_id=tile_id,
                node=root,
            )
        ),
        types_pb2.MutationProto(
            add_node=types_pb2.AddNodeMutation(
                tile_id=tile_id,
                parent_id=root.id,
                node=avatar,
            )
        ),
        types_pb2.MutationProto(
            add_node=types_pb2.AddNodeMutation(
                tile_id=tile_id,
                parent_id=root.id,
                node=text,
            )
        ),
    ]
    return root, avatar, text, mutations


def build_presence_card_tree_mutations(
    tile_id: bytes,
    resource_id: Any,
    agent_name: str,
    last_active_label: str = "now",
) -> tuple[types_pb2.NodeProto, types_pb2.NodeProto, types_pb2.NodeProto, list[types_pb2.MutationProto]]:
    """Alias for build_presence_card_add_node_mutations()."""
    return build_presence_card_add_node_mutations(
        tile_id=tile_id,
        resource_id=resource_id,
        agent_name=agent_name,
        last_active_label=last_active_label,
    )


def _make_node(data: dict) -> types_pb2.NodeProto:
    """Build a NodeProto from a dict spec.

    Supported types:
      {"solid_color": {"r": f, "g": f, "b": f, "a": f}, "bounds": [x,y,w,h]}
      {"text_markdown": {"content": str, "font_size_px": f, "color": [r,g,b,a]}, "bounds": [x,y,w,h]}
      {"hit_region": {"interaction_id": str, "accepts_focus": bool, "accepts_pointer": bool}, "bounds": [x,y,w,h]}
    """
    node = types_pb2.NodeProto(id=_uuid_bytes())

    bounds = data.get("bounds", [0, 0, 100, 100])
    rect = types_pb2.Rect(x=bounds[0], y=bounds[1], width=bounds[2], height=bounds[3])

    if "solid_color" in data:
        c = data["solid_color"]
        node.solid_color.CopyFrom(types_pb2.SolidColorNodeProto(
            color=types_pb2.Rgba(r=c["r"], g=c["g"], b=c["b"], a=c.get("a", 1.0)),
            bounds=rect,
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
    ):
        self.target = target
        self.psk = psk
        self.agent_id = agent_id
        self.capabilities = capabilities or [
            "create_tiles",
            "modify_own_tiles",
            "access_input_events",
        ]
        self._channel: Optional[grpc.aio.Channel] = None
        self._stream = None
        self._seq = 0
        self._server_seq = 0
        self.session_id: Optional[bytes] = None
        self.namespace: Optional[str] = None
        self.granted_capabilities: list[str] = []
        self._response_queue: asyncio.Queue = asyncio.Queue()
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
                initial_subscriptions=["SCENE_TOPOLOGY"],
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
        self.granted_capabilities = list(est.granted_capabilities)
        print(f"  [grpc] Session established: namespace={self.namespace}, "
              f"caps={self.granted_capabilities}", flush=True)

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
                await self._response_queue.put(msg)
        except grpc.aio.AioRpcError as e:
            if e.code() != grpc.StatusCode.CANCELLED:
                print(f"  [grpc] Stream error: {e}", flush=True)
        except Exception as e:
            print(f"  [grpc] Reader error: {e}", flush=True)

    async def _wait_for(self, payload_name: str, timeout: float = 10.0) -> Any:
        """Wait for a ServerMessage with the given payload type."""
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"Timed out waiting for {payload_name}")
            try:
                msg = await asyncio.wait_for(
                    self._response_queue.get(), timeout=remaining
                )
            except asyncio.TimeoutError:
                raise TimeoutError(f"Timed out waiting for {payload_name}")

            which = msg.WhichOneof("payload")
            if which == payload_name:
                return msg
            elif which == "session_error":
                raise RuntimeError(
                    f"Session error: {msg.session_error.code} — "
                    f"{msg.session_error.message} (hint: {msg.session_error.hint})"
                )
            # else: ignore other messages (scene snapshots, deltas, etc.)

    async def wait_for(self, payload_name: str, timeout: float = 10.0) -> Any:
        """Public wrapper for waiting on a specific server payload."""
        return await self._wait_for(payload_name, timeout)

    async def _send(self, **payload_kwargs):
        """Send a ClientMessage with the given payload field."""
        msg = session_pb2.ClientMessage(
            sequence=self._next_seq(),
            timestamp_wall_us=_now_wall_us(),
            **payload_kwargs,
        )
        if self._send_queue is None:
            raise RuntimeError("client transport has not been initialized")
        await self._send_queue.put(msg)

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
        """Disconnect the session, either gracefully or by dropping transport."""
        if graceful:
            await self.session_close(reason=reason, expect_resume=expect_resume)
            return
        await self.drop_connection()

    async def release_lease(self, lease_id: bytes):
        """Release a lease, removing all its tiles immediately."""
        await self._send(
            lease_release=session_pb2.LeaseRelease(lease_id=lease_id)
        )
        resp = await self._wait_for("lease_response", timeout=5.0)
        print(f"  [grpc] Lease released", flush=True)

    async def close(self):
        """Gracefully close the session."""
        try:
            if not self._session_close_sent and not self._transport_closed:
                await self.session_close()
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
            raise RuntimeError(f"Lease denied: {lr.denial_reason}")
        print(f"  [grpc] Lease granted: ttl={lr.granted_ttl_ms}ms, "
              f"priority={lr.granted_priority}", flush=True)
        return lr.lease_id

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
        resp = await self._wait_for("mutation_result", timeout=timeout)
        mr = resp.mutation_result
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
        x: float = 16.0,
        y: float = 0.0,
        w: float = 200.0,
        h: float = 80.0,
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
            types_pb2.TILE_INPUT_MODE_PASSTHROUGH,
        )
        root, avatar, text, _ = build_presence_card_add_node_mutations(
            tile_id=tile_id,
            resource_id=avatar_resource_id,
            agent_name=agent_name,
        )
        await self.set_tile_root(lease_id, tile_id, root)
        await self.add_node(lease_id, tile_id, avatar, parent_id=root.id)
        await self.add_node(lease_id, tile_id, text, parent_id=root.id)
        return tile_id

    async def send_heartbeat(self):
        """Send a keepalive heartbeat."""
        await self._send(heartbeat=session_pb2.Heartbeat())


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
        avatar_resource_id = upload_avatar_png(avatar_png)
        tile_id = await client.create_presence_card_tile(
            lease_id,
            tab_id=None,
            agent_name="grpc-self-test",
            avatar_resource_id=avatar_resource_id,
            x=500,
            y=300,
            w=200,
            h=80,
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
