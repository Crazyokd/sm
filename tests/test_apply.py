import json
import os

import tomlkit

from sm.apply import apply_writes, plan_apply
from sm.catalog import Catalog, OwnedWrite
from sm.state import State


def write(target, content=b"managed", rid="review", mode="file"):
    return OwnedWrite(rid, target, mode, content, [rid])


def test_rule_preserves_unmanaged_text_and_removes_own_block(tmp_path):
    target = tmp_path / "AGENTS.md"
    target.write_text("Existing rules\n")
    state = State(tmp_path / "state.db")
    report = apply_writes([write(target, b"New rules", mode="markdown_block")], state)
    assert report.results["review"]["status"] == "applied"
    assert target.read_text().startswith("Existing rules\n")
    apply_writes([], state)
    assert target.read_text() == "Existing rules\n"


def test_mcp_merge_preserves_unowned_credentials(tmp_path):
    target = tmp_path / "config.toml"
    target.write_text('[model_providers.local]\napi_key="local-secret"\n[mcp_servers.keep]\ncommand="keep"\n')
    state = State(tmp_path / "state.db")
    content = {"mcp_servers": {"docs": {"command": "docs"}}}
    apply_writes([write(target, content, mode="toml_entries")], state)
    data = tomlkit.parse(target.read_text())
    assert data["model_providers"]["local"]["api_key"] == "local-secret"
    assert set(data["mcp_servers"]) == {"keep", "docs"}
    apply_writes([], state)
    assert set(tomlkit.parse(target.read_text())["mcp_servers"]) == {"keep"}


def test_codex_hook_merge_preserves_existing_groups(tmp_path):
    target = tmp_path / "hooks.json"
    keep = {"hooks": [{"type": "command", "command": "keep"}]}
    own = {"hooks": [{"type": "command", "command": "managed"}]}
    target.write_text(json.dumps({"hooks": {"Stop": [keep]}}))
    state = State(tmp_path / "state.db")
    apply_writes([write(target, {"hooks": {"Stop": [own]}}, mode="json_entries")], state)
    assert json.loads(target.read_text())["hooks"]["Stop"] == [keep, own]
    apply_writes([], state)
    assert json.loads(target.read_text())["hooks"]["Stop"] == [keep]


def test_kimi_hook_merge_preserves_existing_entries(tmp_path):
    target = tmp_path / "config.toml"
    target.write_text('[[hooks]]\nevent="Stop"\ncommand="keep"\n')
    state = State(tmp_path / "state.db")
    apply_writes([write(target, {"hooks": [{"event": "Stop", "command": "new"}]}, mode="toml_entries")], state)
    apply_writes([], state)
    assert tomlkit.parse(target.read_text())["hooks"][0]["command"] == "keep"


def test_local_edits_are_not_overwritten_or_deleted(tmp_path):
    target = tmp_path / "SKILL.md"
    state = State(tmp_path / "state.db")
    apply_writes([write(target)], state)
    target.write_bytes(b"local edit")
    assert apply_writes([write(target, b"new")], state).results["review"]["status"] == "conflict"
    assert apply_writes([], state).results["review"]["status"] == "conflict"
    assert target.read_bytes() == b"local edit"


def test_existing_unmanaged_file_is_never_adopted(tmp_path):
    target = tmp_path / "SKILL.md"
    target.write_bytes(b"mine")
    report = apply_writes([write(target)], State(tmp_path / "state.db"))
    assert report.results["review"]["status"] == "conflict"
    assert target.read_bytes() == b"mine"


def test_move_removes_old_managed_location(tmp_path):
    old, new = tmp_path / "old/SKILL.md", tmp_path / "new/SKILL.md"
    state = State(tmp_path / "state.db")
    apply_writes([write(old)], state)
    apply_writes([write(new, b"new")], state)
    assert not old.exists()
    assert new.read_bytes() == b"new"


def test_conflicting_move_keeps_previous_managed_location(tmp_path):
    old, new = tmp_path / "a-old/SKILL.md", tmp_path / "z-new/SKILL.md"
    state = State(tmp_path / "state.db")
    apply_writes([write(old)], state)
    new.parent.mkdir()
    new.write_bytes(b"someone else")
    assert apply_writes([write(new, b"new")], state).results["review"]["status"] == "conflict"
    assert old.read_bytes() == b"managed"


def test_target_symlink_never_follows_outside(tmp_path):
    outside = tmp_path / "outside"
    outside.mkdir()
    link = tmp_path / "linked"
    link.symlink_to(outside, target_is_directory=True)
    report = apply_writes([write(link / "file")], State(tmp_path / "state.db"))
    assert report.results["review"]["status"] == "conflict"
    assert not (outside / "file").exists()


def test_interrupted_replace_keeps_previous_file(tmp_path, monkeypatch):
    target = tmp_path / "file"
    state = State(tmp_path / "state.db")
    apply_writes([write(target)], state)
    def fail(*args):
        raise OSError("interrupted")
    monkeypatch.setattr(os, "replace", fail)
    assert apply_writes([write(target, b"new")], state).results["review"]["status"] == "failed"
    assert target.read_bytes() == b"managed"


def test_portable_skill_has_one_shared_installation(tmp_path):
    from sm.catalog import Resource
    payload = tmp_path / "resources/review"
    payload.mkdir(parents=True)
    (payload / "SKILL.md").write_text("Portable content")
    entry = Resource("review", "skill", "self", "user", None, payload="resources/review")
    state = State(tmp_path / "state.db")
    writes = plan_apply(Catalog({}, [entry]), {"review": tmp_path}, tmp_path / "home", state)
    assert len(writes) == 1
    assert writes[0].path == tmp_path / "home/.agents/skills/review/SKILL.md"


def test_wrong_config_container_preserves_file_and_other_resources(tmp_path):
    target = tmp_path / "hooks.json"
    target.write_text('{"hooks":[]}')
    own = {"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "managed"}]}]}}
    good = tmp_path / "good"
    report = apply_writes([write(target, own, mode="json_entries"), write(good, b"good", rid="other")], State(tmp_path / "state.db"))
    assert report.results["review"]["status"] == "conflict"
    assert target.read_text() == '{"hooks":[]}'
    assert good.read_bytes() == b"good"


def test_approved_executable_changes_are_applied_with_or_without_content_change(tmp_path):
    import stat
    target = tmp_path / "run.sh"
    state = State(tmp_path / "state.db")
    apply_writes([write(target)], state)
    for content, executable in ((b"managed", True), (b"changed", False), (b"changed again", True)):
        desired = write(target, content)
        desired.executable = executable
        assert apply_writes([desired], state).ok
        assert bool(target.stat().st_mode & stat.S_IXUSR) is executable
