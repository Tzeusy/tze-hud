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
        tile_id = await client.create_tile(lease_id, x=50, y=50, w=400, h=300)
        await client.set_tile_root(lease_id, tile_id, node)
"""

from __future__ import annotations

import asyncio
import os
import sys
import time
import uuid
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
        self._channel = grpc.aio.insecure_channel(self.target)
        stub = session_pb2_grpc.HudSessionStub(self._channel)

        # Build the outbound request iterator — we'll feed messages via a queue.
        self._send_queue: asyncio.Queue = asyncio.Queue()
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

    async def _send(self, **payload_kwargs):
        """Send a ClientMessage with the given payload field."""
        msg = session_pb2.ClientMessage(
            sequence=self._next_seq(),
            timestamp_wall_us=_now_wall_us(),
            **payload_kwargs,
        )
        await self._send_queue.put(msg)

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
            await self._send(
                session_close=session_pb2.SessionClose(reason="test complete")
            )
        except Exception:
            pass
        if self._send_queue:
            await self._send_queue.put(None)  # Signal iterator to stop
        if self._reader_task:
            self._reader_task.cancel()
            try:
                await self._reader_task
            except (asyncio.CancelledError, Exception):
                pass
        if self._channel:
            await self._channel.close()

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

    async def create_tile(
        self,
        lease_id: bytes,
        x: float = 50,
        y: float = 50,
        w: float = 400,
        h: float = 300,
        z_order: int = 100,
    ) -> bytes:
        """Create a tile and return its SceneId."""
        batch_id = _uuid_bytes()
        await self._send(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=[
                    types_pb2.MutationProto(
                        create_tile=types_pb2.CreateTileMutation(
                            bounds=types_pb2.Rect(x=x, y=y, width=w, height=h),
                            z_order=z_order,
                        )
                    )
                ],
            )
        )
        resp = await self._wait_for("mutation_result", timeout=5.0)
        mr = resp.mutation_result
        if not mr.accepted:
            raise RuntimeError(f"CreateTile rejected: {mr.error_code} — {mr.error_message}")
        tile_id = mr.created_ids[0]
        print(f"  [grpc] Tile created: {tile_id.hex()[:16]}...", flush=True)
        return tile_id

    async def set_tile_root(
        self,
        lease_id: bytes,
        tile_id: bytes,
        node_spec: dict,
    ):
        """Set a tile's root node from a dict spec."""
        node = _make_node(node_spec)
        batch_id = _uuid_bytes()
        await self._send(
            mutation_batch=session_pb2.MutationBatch(
                batch_id=batch_id,
                lease_id=lease_id,
                mutations=[
                    types_pb2.MutationProto(
                        set_tile_root=types_pb2.SetTileRootMutation(
                            tile_id=tile_id,
                            node=node,
                        )
                    )
                ],
            )
        )
        resp = await self._wait_for("mutation_result", timeout=5.0)
        mr = resp.mutation_result
        if not mr.accepted:
            raise RuntimeError(f"SetTileRoot rejected: {mr.error_code} — {mr.error_message}")
        print(f"  [grpc] Tile root set", flush=True)

    async def send_heartbeat(self):
        """Send a keepalive heartbeat."""
        await self._send(heartbeat=session_pb2.Heartbeat())


# ---------------------------------------------------------------------------
# Quick self-test
# ---------------------------------------------------------------------------

async def _self_test():
    """Connect, create a tile with a colored background, hold for 5s, close."""
    import argparse
    parser = argparse.ArgumentParser(description="gRPC client self-test")
    parser.add_argument("--target", default="tzehouse-windows.parrot-hen.ts.net:50051")
    parser.add_argument("--psk", default=os.getenv("MCP_TEST_PSK", "tze-hud-key"))
    args = parser.parse_args()

    print(f"Connecting to {args.target}...", flush=True)
    async with HudClient(args.target, psk=args.psk, agent_id="grpc-self-test") as client:
        lease_id = await client.request_lease(ttl_ms=30000)
        tile_id = await client.create_tile(lease_id, x=500, y=300, w=500, h=400)
        await client.set_tile_root(lease_id, tile_id, {
            "solid_color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0},
            "bounds": [0, 0, 500, 400],
        })
        print("  Big red tile at (500,300) 500x400 — visible for 10 seconds...", flush=True)
        await asyncio.sleep(10)
        await client.release_lease(lease_id)
        print("  Closing session.", flush=True)


if __name__ == "__main__":
    asyncio.run(_self_test())
