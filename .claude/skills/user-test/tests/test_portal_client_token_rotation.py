"""Regression tests for owner-token rotation in the preferred portal client."""

from __future__ import annotations

import importlib.util
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import pytest


CLIENT_PATH = (
    Path(__file__).parents[2] / "hud-projection" / "scripts" / "portal_client.py"
)
SPEC = importlib.util.spec_from_file_location("portal_client_token_rotation", CLIENT_PATH)
assert SPEC is not None and SPEC.loader is not None
portal_client = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(portal_client)


def attach_args() -> SimpleNamespace:
    return SimpleNamespace(
        projection_id="review-session",
        provider_kind="codex",
        display_name="Review Session",
        classification="private",
        idempotency_key="stable-attach-key",
        workspace_hint=None,
        repository_hint=None,
        icon_profile=None,
    )


def test_matching_replay_replaces_stale_token_file(tmp_path: Path) -> None:
    portal_client.TOKEN_DIR = str(tmp_path)
    token_file = Path(portal_client.token_path("review-session"))
    token_file.write_text("stale-token", encoding="utf-8")
    response = {
        "result": {
            "accepted": True,
            "owner_token": "fresh-token",
            "status_summary": "projection ownership refreshed",
        }
    }

    with mock.patch.object(portal_client, "call_tool", return_value=response), mock.patch.object(
        portal_client, "emit"
    ):
        portal_client.cmd_attach(attach_args())

    assert token_file.read_text(encoding="utf-8") == "fresh-token"


def test_accepted_replay_without_token_fails_instead_of_reusing_stale_token(
    tmp_path: Path,
) -> None:
    portal_client.TOKEN_DIR = str(tmp_path)
    token_file = Path(portal_client.token_path("review-session"))
    token_file.write_text("stale-token", encoding="utf-8")
    response = {"result": {"accepted": True, "owner_token": None}}

    with mock.patch.object(portal_client, "call_tool", return_value=response), pytest.raises(
        SystemExit
    ) as exc:
        portal_client.cmd_attach(attach_args())

    assert exc.value.code == 2
    assert token_file.read_text(encoding="utf-8") == "stale-token"


def test_attach_conflict_is_not_masked_by_existing_token_file(tmp_path: Path) -> None:
    portal_client.TOKEN_DIR = str(tmp_path)
    Path(portal_client.token_path("review-session")).write_text(
        "current-token", encoding="utf-8"
    )
    response = {
        "error": {
            "code": -32603,
            "data": {"error_code": "PROJECTION_ALREADY_ATTACHED"},
        }
    }

    with mock.patch.object(portal_client, "call_tool", return_value=response), pytest.raises(
        SystemExit
    ) as exc:
        portal_client.cmd_attach(attach_args())

    assert exc.value.code == 2
