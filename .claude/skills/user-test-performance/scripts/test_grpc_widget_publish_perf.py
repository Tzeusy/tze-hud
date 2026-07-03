#!/usr/bin/env python3
"""Focused tests for the grpc_widget_publish_perf SessionClient lease renewal.

Parity follow-up to hud-hk8kl/#1010: the user-test HudClient gained a
``renew_lease()`` so long-lived, lease-driven paths never hit the runtime's
"lease expired" rejection mid-run. These tests prove the perf-skill gRPC client
(``SessionClient``) has an equivalent, working ``renew_lease()`` — even though
its current WidgetPublish benchmark is capability-based and drives no lease.

The tests exercise ``renew_lease`` end-to-end without a live server by seeding
the client's recv queue with a canned ``lease_response`` and reading back the
``lease_renew`` message it enqueues on the send queue.
"""

from __future__ import annotations

import asyncio
import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
if str(SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPT_DIR))

SCRIPT = SCRIPT_DIR / "grpc_widget_publish_perf.py"
SPEC = importlib.util.spec_from_file_location("grpc_widget_publish_perf", SCRIPT)
assert SPEC is not None
perf = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(perf)

session_pb2 = perf.session_pb2


def _lease_response(
    granted: bool,
    ttl_ms: int = 0,
    deny_reason: str = "",
    deny_code: str = "",
) -> "session_pb2.ServerMessage":
    return session_pb2.ServerMessage(
        sequence=1,
        lease_response=session_pb2.LeaseResponse(
            granted=granted,
            granted_ttl_ms=ttl_ms,
            deny_reason=deny_reason,
            deny_code=deny_code,
        ),
    )


class RenewLeaseTests(unittest.TestCase):
    def test_renew_lease_sends_lease_renew_and_records_ttl(self) -> None:
        async def scenario() -> None:
            client = perf.SessionClient("dummy:0")
            client.recv_queue.put_nowait(_lease_response(True, ttl_ms=45_000))

            granted = await client.renew_lease(b"lease-id-16byte0", new_ttl_ms=60_000)

            self.assertEqual(granted, 45_000)
            self.assertEqual(client.last_granted_lease_ttl_ms, 45_000)

            sent = client.send_queue.get_nowait()
            self.assertEqual(sent.WhichOneof("payload"), "lease_renew")
            self.assertEqual(sent.lease_renew.lease_id, b"lease-id-16byte0")
            self.assertEqual(sent.lease_renew.new_ttl_ms, 60_000)

        asyncio.run(scenario())

    def test_renew_lease_defaults_to_reissue_original_ttl(self) -> None:
        async def scenario() -> None:
            client = perf.SessionClient("dummy:0")
            client.recv_queue.put_nowait(_lease_response(True, ttl_ms=30_000))

            granted = await client.renew_lease(b"lease-id-16byte0")

            self.assertEqual(granted, 30_000)
            sent = client.send_queue.get_nowait()
            self.assertEqual(sent.lease_renew.new_ttl_ms, 0)

        asyncio.run(scenario())

    def test_renew_lease_raises_on_denial_without_clobbering_ttl(self) -> None:
        async def scenario() -> None:
            client = perf.SessionClient("dummy:0")
            client.last_granted_lease_ttl_ms = 45_000
            client.recv_queue.put_nowait(
                _lease_response(False, deny_reason="no capacity", deny_code="DENY_BUDGET")
            )

            with self.assertRaises(RuntimeError):
                await client.renew_lease(b"lease-id-16byte0", new_ttl_ms=0)

            # A denied renew must not overwrite the last-known-good TTL.
            self.assertEqual(client.last_granted_lease_ttl_ms, 45_000)

        asyncio.run(scenario())


if __name__ == "__main__":
    unittest.main()
