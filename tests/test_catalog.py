import json
from pathlib import Path

import pytest

from sm.catalog import Resource, UnsupportedResource, load_catalog
from sm.paths import resolve_scope
from sm.providers import codex, kimi


def resource(kind="skill", agent="codex", **changes):
    values = dict(id="review", kind=kind, origin="self", scope="user", project=None)
    values.update({"payload": "resources/review"} if kind == "skill" else
                  {"agents": [agent], "payloads": {agent: "resources/review"}})
    values.update(changes)
    return Resource(**values)


def test_project_scope_requires_project(tmp_path):
    entry = dict(id="review", kind="skill", origin="self", scope="project", project=None,
                 payload="resources/review")
    (tmp_path / "catalog.json").write_text(json.dumps({"projects": {}, "resources": [entry]}))
    with pytest.raises(ValueError, match="project"):
        load_catalog(tmp_path)


@pytest.mark.parametrize("changes", [
    {"agents": ["unknown"]}, {"kind": "plugin"}, {"origin": "other"},
    {"scope": "system"}, {"id": "../escape"},
    {"payload": "../outside"}, {"payload": "/absolute"},
])
def test_invalid_metadata_is_rejected(changes):
    with pytest.raises(ValueError):
        resource(**changes)


def test_project_resolves_under_each_home(tmp_path):
    entry = resource(scope="project", project="app")
    from sm.catalog import Catalog
    catalog = Catalog(projects={"app": "~/repo/app"}, resources=[entry])
    for machine in ("one", "two"):
        home = tmp_path / machine
        assert resolve_scope(entry, catalog, home) == home / "repo/app"


@pytest.mark.parametrize("provider,agent,base", [
    (codex, "codex", ".agents"), (kimi, "kimi", ".agents"),
])
def test_skill_targets_keep_scope(provider, agent, base, tmp_path):
    payload = tmp_path / "payload"
    payload.mkdir()
    (payload / "SKILL.md").write_text("---\nname: review\ndescription: Review code\n---\nRead code.")
    (payload / "references").mkdir()
    (payload / "references/check.md").write_text("Check behavior.")
    home = tmp_path / "home"
    writes = provider.render(resource(agent=agent), payload, home)
    assert {w.path for w in writes} == {
        home / base / "skills/review/SKILL.md", home / base / "skills/review/references/check.md"}
    assert all(w.resource_id == "review" for w in writes)


@pytest.mark.parametrize("provider,agent,kind,filename,body,target", [
    (codex, "codex", "rule", "AGENTS.md", "Keep changes narrow.", ".codex/AGENTS.md"),
    (kimi, "kimi", "rule", "AGENTS.md", "Keep changes narrow.", ".kimi-code/AGENTS.md"),
    (codex, "codex", "agent", "agent.toml", 'name = "review"\ndescription = "Review"\ndeveloper_instructions = "Read"', ".codex/agents/review.toml"),
    (kimi, "kimi", "agent", "agent.md", "---\ndescription: Review\n---\nRead", ".kimi-code/agents/review.md"),
    (codex, "codex", "mcp", "config.toml", '[mcp_servers.docs]\ncommand="docs"', ".codex/config.toml"),
    (kimi, "kimi", "mcp", "mcp.json", '{"mcpServers":{"docs":{"command":"docs"}}}', ".kimi-code/mcp.json"),
    (codex, "codex", "hook", "hooks.json", '{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"true"}]}]}}', ".codex/hooks.json"),
    (kimi, "kimi", "hook", "config.toml", '[[hooks]]\nevent="Stop"\ncommand="true"', ".kimi-code/config.toml"),
])
def test_native_resource_targets(provider, agent, kind, filename, body, target, tmp_path):
    payload = tmp_path / "payload"
    payload.mkdir()
    (payload / filename).write_text(body)
    home = tmp_path / "home"
    writes = provider.render(resource(kind, agent), payload, home)
    assert any(w.path == home / target for w in writes)


def test_project_rule_targets_root_agents_file(tmp_path):
    payload = tmp_path / "payload"
    payload.mkdir()
    (payload / "AGENTS.md").write_text("Project rules")
    writes = codex.render(resource("rule", scope="project", project="app"), payload, tmp_path / "app")
    assert writes[0].path == tmp_path / "app/AGENTS.md"


def test_kimi_project_hook_not_promoted_to_user(tmp_path):
    with pytest.raises(UnsupportedResource, match="project"):
        kimi.render(resource("hook", "kimi", scope="project", project="app"), tmp_path, tmp_path)


def test_payload_symlink_escape_rejected(tmp_path):
    payload = tmp_path / "payload"
    payload.mkdir()
    (payload / "SKILL.md").symlink_to("/etc/passwd")
    with pytest.raises(ValueError, match="symlink"):
        codex.render(resource(), payload, tmp_path / "home")


def test_skill_definition_has_one_payload_and_no_client_classification():
    entry = Resource(id="portable", kind="skill", origin="self", scope="user", project=None,
                     payload="resources/portable")
    assert entry.targets == {"codex": "resources/portable", "kimi": "resources/portable"}
    assert not entry.agents


def test_skill_rejects_client_specific_definitions():
    with pytest.raises(ValueError, match="client"):
        Resource(id="portable", kind="skill", origin="self", scope="user", project=None,
                 agents=["codex"], payloads={"codex": "resources/portable"})
