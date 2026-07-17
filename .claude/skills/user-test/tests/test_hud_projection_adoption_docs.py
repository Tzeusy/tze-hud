"""Contract checks for the HUD projection adoption path."""

from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
SKILL_DIR = ROOT / ".claude" / "skills" / "hud-projection"


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_standard_mcp_dialect_is_primary_across_projection_guidance() -> None:
    skill = read(SKILL_DIR / "SKILL.md")
    facade = read(SKILL_DIR / "references" / "mcp-facade.md")
    examples = read(SKILL_DIR / "references" / "operation-examples.md")
    adapter = read(SKILL_DIR / "agents" / "openai.yaml")

    for contents in (skill, facade, examples, adapter):
        assert "tools/call" in contents
    assert "primary" in skill.lower()
    assert "legacy" in skill.lower()
    assert "does not implement standard `tools/call`" not in skill
    assert "bare-method first" not in skill


def test_manual_tab_bootstrap_is_only_a_warp_vm_fallback() -> None:
    skill = read(SKILL_DIR / "SKILL.md")
    quickstart = read(ROOT / "docs" / "QUICKSTART.md")

    assert "WARP" in skill
    assert "No active tab" in skill
    assert "normal gpu desktop" in skill.lower()
    assert "until runtime bug" not in skill
    assert "Known runtime bug (hud-d5rcd)" not in quickstart
    assert "not needed on a normal gpu desktop" in quickstart.lower()


def test_projection_guidance_assigns_durable_continuity_to_the_client() -> None:
    skill = read(SKILL_DIR / "SKILL.md")
    facade = read(SKILL_DIR / "references" / "mcp-facade.md")
    examples = read(SKILL_DIR / "references" / "operation-examples.md")

    for contents in (skill, facade, examples):
        assert "portal-continuity" in contents
        assert "logical_unit_id" in contents
        assert "coalesce_key" in contents
    assert "64 records" in skill
    assert "64 KiB" in skill
    assert "continuity-clear" in skill
    assert "viewer-authored" in skill
    assert "runtime restart" in skill.lower()
    assert "durable transcript" in facade.lower()
    assert "owner token" in examples.lower()
