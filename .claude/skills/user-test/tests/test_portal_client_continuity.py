"""Contract tests for client-owned portal continuity replay."""

from __future__ import annotations

import importlib.util
import json
import os
import stat
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import pytest


CLIENT_PATH = (
    Path(__file__).parents[2] / "hud-projection" / "scripts" / "portal_client.py"
)
SPEC = importlib.util.spec_from_file_location("portal_client_continuity", CLIENT_PATH)
assert SPEC is not None and SPEC.loader is not None
portal_client = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(portal_client)


def attach_args(*, idempotency_key: str | None = None) -> SimpleNamespace:
    return SimpleNamespace(
        projection_id="continuity-session",
        provider_kind="codex",
        display_name="Continuity Session",
        classification="private",
        idempotency_key=idempotency_key,
        workspace_hint=None,
        repository_hint=None,
        icon_profile=None,
    )


def publish_args(
    text: str,
    *,
    kind: str = "assistant",
    classification: str = "private",
    logical_unit_id: str | None = None,
    coalesce_key: str | None = None,
) -> SimpleNamespace:
    return SimpleNamespace(
        projection_id="continuity-session",
        text=text,
        text_file=None,
        kind=kind,
        classification=classification,
        logical_unit_id=logical_unit_id,
        coalesce_key=coalesce_key,
    )


@pytest.fixture(autouse=True)
def private_state_dirs(tmp_path: Path) -> None:
    portal_client.TOKEN_DIR = str(tmp_path / "tokens")
    portal_client.CONTINUITY_DIR = str(tmp_path / "continuity")


def record(unit: str, text: str, *, coalesce_key: str | None = None) -> dict:
    item = {
        "output_text": text,
        "output_kind": "assistant",
        "content_classification": "private",
        "logical_unit_id": unit,
    }
    if coalesce_key is not None:
        item["coalesce_key"] = coalesce_key
    return item


def test_continuity_state_is_private_and_atomically_replaced() -> None:
    state = {
        "version": 1,
        "idempotency_key": "stable-attach-key",
        "records": [record("turn-1", "hello")],
    }

    real_replace = os.replace
    with mock.patch.object(os, "replace", wraps=real_replace) as replace:
        portal_client.save_continuity("continuity-session", state)

    path = Path(portal_client.continuity_path("continuity-session"))
    assert stat.S_IMODE(path.parent.stat().st_mode) == 0o700
    assert stat.S_IMODE(path.stat().st_mode) == 0o600
    assert replace.call_count == 1
    assert replace.call_args.args[1] == str(path)
    assert not list(path.parent.glob("*.tmp-*"))


def test_rolling_tail_enforces_item_and_canonical_utf8_byte_bounds() -> None:
    records = [record(f"turn-{index}", "é" * index) for index in range(1, 5)]

    item_bounded = portal_client.bound_records(
        records, max_items=3, max_bytes=1_000_000
    )
    assert [item["logical_unit_id"] for item in item_bounded] == [
        "turn-2",
        "turn-3",
        "turn-4",
    ]

    last_two_bytes = sum(portal_client.record_size(item) for item in records[-2:])
    byte_bounded = portal_client.bound_records(
        records, max_items=10, max_bytes=last_two_bytes
    )
    assert [item["logical_unit_id"] for item in byte_bounded] == [
        "turn-3",
        "turn-4",
    ]
    assert (
        sum(portal_client.record_size(item) for item in byte_bounded) <= last_two_bytes
    )


def test_generated_identity_keys_respect_the_128_byte_wire_contract() -> None:
    projection_id = "p" * 128

    logical_unit_id = portal_client.new_logical_unit_id(projection_id)
    idempotency_key = portal_client._idempotency_key_for_attach(
        projection_id, None, portal_client.empty_continuity()
    )

    assert len(logical_unit_id.encode("utf-8")) <= 128
    assert len(idempotency_key.encode("utf-8")) <= 128


def test_same_coalesce_key_replaces_local_tail_entry_in_place() -> None:
    state = {
        "version": 1,
        "idempotency_key": "stable-attach-key",
        "records": [
            record("turn-1", "first"),
            record("progress-1", "10%", coalesce_key="progress"),
            record("turn-2", "second"),
        ],
    }

    updated = portal_client.retain_record(
        state, record("progress-2", "80%", coalesce_key="progress")
    )

    assert [item["logical_unit_id"] for item in updated["records"]] == [
        "turn-1",
        "progress-2",
        "turn-2",
    ]


def test_repeated_logical_unit_id_preserves_original_record_once() -> None:
    original = record("turn-1", "first payload")
    state = {
        "version": 1,
        "idempotency_key": "stable-attach-key",
        "records": [original],
    }

    updated = portal_client.retain_record(
        state,
        record("turn-1", "retry payload must remain an authority no-op"),
    )

    assert updated["records"] == [original]


def test_corrupt_state_is_quarantined_without_replaying_untrusted_content(
    capsys: pytest.CaptureFixture[str],
) -> None:
    path = Path(portal_client.continuity_path("continuity-session"))
    path.parent.mkdir(mode=0o700, parents=True)
    path.write_text('{"records":[', encoding="utf-8")
    path.chmod(0o600)

    loaded = portal_client.load_continuity("continuity-session")

    assert loaded == portal_client.empty_continuity()
    assert not path.exists()
    quarantined = path.with_suffix(path.suffix + ".corrupt")
    assert quarantined.read_text(encoding="utf-8") == '{"records":['
    assert stat.S_IMODE(quarantined.stat().st_mode) == 0o600
    assert "quarantined corrupt continuity state" in capsys.readouterr().err


def test_published_record_preserves_semantics_without_persisting_owner_token() -> None:
    portal_client.save_token("continuity-session", "owner-token-must-not-leak")
    response = {"result": {"accepted": True, "status_summary": "published"}}

    with (
        mock.patch.object(portal_client, "call_tool", return_value=response),
        mock.patch.object(portal_client, "emit"),
        mock.patch.object(
            portal_client, "new_logical_unit_id", return_value="stable-unit"
        ),
    ):
        portal_client.cmd_publish(
            publish_args(
                "authored text",
                kind="tool",
                classification="household",
                coalesce_key="tool-progress",
            )
        )

    path = Path(portal_client.continuity_path("continuity-session"))
    raw = path.read_text(encoding="utf-8")
    assert "owner-token-must-not-leak" not in raw
    assert portal_client.load_continuity("continuity-session")["records"] == [
        {
            "output_text": "authored text",
            "output_kind": "tool",
            "content_classification": "household",
            "logical_unit_id": "stable-unit",
            "coalesce_key": "tool-progress",
        }
    ]


def test_attach_reuses_original_key_and_double_replay_is_idempotent() -> None:
    portal_client.save_continuity(
        "continuity-session",
        {
            "version": 1,
            "idempotency_key": "original-key",
            "records": [record("turn-1", "one"), record("turn-2", "two")],
        },
    )
    applied: dict[str, dict] = {}
    attach_keys: list[str] = []

    def fake_call(tool: str, args: dict) -> dict:
        if tool == "portal_projection_attach":
            attach_keys.append(args["idempotency_key"])
            return {"result": {"accepted": True, "owner_token": "rotated-token"}}
        if tool == "portal_projection_publish":
            applied.setdefault(args["logical_unit_id"], args.copy())
            return {"result": {"accepted": True}}
        raise AssertionError(tool)

    with (
        mock.patch.object(portal_client, "call_tool", side_effect=fake_call),
        mock.patch.object(portal_client, "emit"),
    ):
        portal_client.cmd_attach(attach_args())
        portal_client.cmd_attach(attach_args())

    assert attach_keys == ["original-key", "original-key"]
    assert list(applied) == ["turn-1", "turn-2"]
    assert portal_client.load_continuity("continuity-session")["records"] == [
        record("turn-1", "one"),
        record("turn-2", "two"),
    ]


def test_fresh_runtime_attach_replays_authored_tail_with_original_semantics() -> None:
    retained = [
        record("turn-7", "answer"),
        {
            **record("progress-9", "done", coalesce_key="progress"),
            "output_kind": "status",
            "content_classification": "household",
        },
    ]
    portal_client.save_continuity(
        "continuity-session",
        {"version": 1, "idempotency_key": "original-key", "records": retained},
    )
    fresh_runtime: list[dict] = []

    def fake_call(tool: str, args: dict) -> dict:
        if tool == "portal_projection_attach":
            return {"result": {"accepted": True, "owner_token": "fresh-runtime-token"}}
        if tool == "portal_projection_publish":
            fresh_runtime.append(args.copy())
            return {"result": {"accepted": True}}
        raise AssertionError(tool)

    with (
        mock.patch.object(portal_client, "call_tool", side_effect=fake_call),
        mock.patch.object(portal_client, "emit"),
    ):
        portal_client.cmd_attach(attach_args())

    replayed = [
        {
            key: value
            for key, value in item.items()
            if key
            in {
                "output_text",
                "output_kind",
                "content_classification",
                "logical_unit_id",
                "coalesce_key",
            }
        }
        for item in fresh_runtime
    ]
    assert replayed == retained
    assert all(item["owner_token"] == "fresh-runtime-token" for item in fresh_runtime)


def test_rejected_publish_rolls_back_prepared_local_record() -> None:
    initial = {
        "version": 1,
        "idempotency_key": "original-key",
        "records": [record("turn-1", "committed")],
    }
    portal_client.save_continuity("continuity-session", initial)
    portal_client.save_token("continuity-session", "owner-token")

    with (
        mock.patch.object(
            portal_client,
            "call_tool",
            return_value={"error": {"code": -32105, "message": "too large"}},
        ),
        mock.patch.object(portal_client, "new_logical_unit_id", return_value="turn-2"),
        pytest.raises(SystemExit) as exc,
    ):
        portal_client.cmd_publish(publish_args("rejected"))

    assert exc.value.code == 2
    assert portal_client.load_continuity("continuity-session") == initial


def test_ambiguous_transport_failure_retains_record_for_idempotent_replay() -> None:
    initial = {
        "version": 1,
        "idempotency_key": "original-key",
        "records": [record("turn-1", "committed")],
    }
    portal_client.save_continuity("continuity-session", initial)
    portal_client.save_token("continuity-session", "owner-token")

    with (
        mock.patch.object(portal_client, "call_tool", side_effect=SystemExit(1)),
        mock.patch.object(portal_client, "new_logical_unit_id", return_value="turn-2"),
        pytest.raises(SystemExit) as exc,
    ):
        portal_client.cmd_publish(publish_args("acceptance-unknown"))

    assert exc.value.code == 1
    assert portal_client.load_continuity("continuity-session") == {
        **initial,
        "records": [
            record("turn-1", "committed"),
            record("turn-2", "acceptance-unknown"),
        ],
    }


def test_failed_atomic_replace_preserves_previous_state() -> None:
    initial = {
        "version": 1,
        "idempotency_key": "original-key",
        "records": [record("turn-1", "committed")],
    }
    portal_client.save_continuity("continuity-session", initial)

    with (
        mock.patch.object(os, "replace", side_effect=OSError("disk unavailable")),
        pytest.raises(OSError, match="disk unavailable"),
    ):
        portal_client.save_continuity(
            "continuity-session",
            portal_client.retain_record(initial, record("turn-2", "new")),
        )

    assert portal_client.load_continuity("continuity-session") == initial
    assert not list(Path(portal_client.CONTINUITY_DIR).glob("*.tmp-*"))


def test_local_continuity_cleanup_is_explicit_and_idempotent() -> None:
    portal_client.save_continuity(
        "continuity-session",
        {
            "version": 1,
            "idempotency_key": "original-key",
            "records": [record("turn-1", "committed")],
        },
    )

    assert portal_client.clear_continuity("continuity-session") is True
    assert portal_client.clear_continuity("continuity-session") is False
    assert not Path(portal_client.continuity_path("continuity-session")).exists()


def test_continuity_json_never_accepts_unknown_or_token_fields() -> None:
    path = Path(portal_client.continuity_path("continuity-session"))
    path.parent.mkdir(mode=0o700, parents=True)
    path.write_text(
        json.dumps(
            {
                "version": 1,
                "idempotency_key": "original-key",
                "owner_token": "forbidden",
                "records": [
                    {
                        **record("turn-1", "safe"),
                        "owner_token": "also-forbidden",
                        "input_items": [{"text": "viewer-private"}],
                    }
                ],
            }
        ),
        encoding="utf-8",
    )
    path.chmod(0o600)

    loaded = portal_client.load_continuity("continuity-session")

    assert loaded == portal_client.empty_continuity()
    assert not path.exists()
