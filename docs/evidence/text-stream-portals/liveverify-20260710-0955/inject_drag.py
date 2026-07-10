#!/usr/bin/env python3
"""Live-verify helper: inject a single pointer drag into the tzehouse overlay
via the exemplar's interactive-session injector. Used to drive a portal RESIZE
(drag the bottom-right resize handle) and a subsequent MOVE (drag the header),
to verify hud-lyqun (group intact after resize+drag) and hud-rpmwt (text
re-wrap after resize). Pointer injection is reliable (unlike keyboard)."""
import asyncio
import os
import sys

SCRIPTS = os.path.join(
    os.path.dirname(__file__),
    "../../../../.claude/skills/user-test/scripts",
)
sys.path.insert(0, os.path.abspath(SCRIPTS))

from text_stream_portal_exemplar import run_windows_diagnostic_input  # noqa: E402

HOST = "windows-host.example"
USER = "admin-user"
KEY = os.path.expanduser("~/.ssh/hud-ssh-key")


async def main() -> None:
    label, sx, sy, ex, ey = sys.argv[1], *map(float, sys.argv[2:6])
    actions = [{
        "kind": "drag", "label": label,
        "start_x": sx, "start_y": sy, "end_x": ex, "end_y": ey,
        "steps": 20,
    }]
    result = await run_windows_diagnostic_input(
        HOST, user=USER, ssh_key=KEY, actions=actions,
        timeout_s=60.0, connect_timeout_s=8.0,
        scene_width=3840.0, scene_height=2160.0,
    )
    print(result)


asyncio.run(main())
