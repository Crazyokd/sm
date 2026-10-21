import json
from pathlib import Path
from unittest.mock import Mock

import pytest

from sm.cli import main
from sm.gitlab import submit_mr
from sm.observe import record_event, pending_observations
from sm.proposals import ChangeSet, create_proposal
from sm.service import cycle, synchronize
from conftest import git


RELATIVE = "repo/app/.agents/skills/review/SKILL.md"


def publish_catalog(repo, catalog):
    (repo / "catalog.json").write_text(json.dumps(catalog))
    git(repo, "add", ".")
    git(repo, "commit", "-m", "Approve catalog adjustment")
    git(repo, "push", "origin", "main")


def test_two_machine_restore_keeps_project_scope(two_machine_case, monkeypatch, capsys):
    case = two_machine_case
    for home in case.homes:
        monkeypatch.setattr(Path, "home", lambda: home)
        assert main(["init", "--repo", str(case.repo)]) == 0
        assert main(["init", "--repo", str(case.repo)]) == 0
        assert main(["sync", "--repo", str(case.repo)]) == 0
    a, b = case.homes
    assert (a / RELATIVE).read_bytes() == (b / RELATIVE).read_bytes()
    for home in case.homes:
        assert not (home / ".agents/skills/review").exists()
        assert not (home / "repo/app/.kimi-code/skills/review").exists()
        assert len(list((home / ".config/systemd/user").glob("sm-*.timer"))) == 1


def test_review_merge_then_next_sync_is_the_only_activation(two_machine_case):
    case = two_machine_case
    home = case.homes[0]
    state = case.state(home)
    synchronize(case.repo, home, state)
    before = (home / RELATIVE).read_text()
    updated = before + "\nApproved improvement.\n"
    change = ChangeSet(["review"], "Improve through review", ["local-reference"],
                       {"resources/review/SKILL.md": updated}, [], {"status": "partial"})
    proposal = create_proposal(case.repo, change, state)
    assert submit_mr(proposal).endswith("/1")
    synchronize(case.repo, home, state)
    assert (home / RELATIVE).read_text() == before
    git(case.repo, "fetch", "origin", proposal.branch)
    git(case.repo, "merge", "--no-ff", "--no-edit", "FETCH_HEAD")
    git(case.repo, "push", "origin", "main")
    assert (home / RELATIVE).read_text() == before
    for machine in case.homes:
        synchronize(case.repo, machine, case.state(machine))
        assert (machine / RELATIVE).read_text() == updated


def test_approved_rename_move_and_retirement_preserve_unmanaged_files(two_machine_case):
    case = two_machine_case
    home = case.homes[0]
    state = case.state(home)
    synchronize(case.repo, home, state)
    keep = home / "repo/app/.agents/skills/personal/SKILL.md"
    keep.parent.mkdir()
    keep.write_text("Not owned by SM")
    catalog = json.loads((case.repo / "catalog.json").read_text())
    entry = catalog["resources"][0]
    git(case.repo, "mv", "resources/review", "resources/review-general")
    entry.update(id="review-general", scope="user", project=None, payload="resources/review-general")
    publish_catalog(case.repo, catalog)
    synchronize(case.repo, home, state)
    assert not (home / RELATIVE).exists()
    assert (home / ".agents/skills/review-general/SKILL.md").exists()
    publish_catalog(case.repo, {"projects": catalog["projects"], "resources": []})
    synchronize(case.repo, home, state)
    assert not (home / ".agents/skills/review-general/SKILL.md").exists()
    assert keep.read_text() == "Not owned by SM"


def test_upstream_follows_directly_and_never_calls_analysis(two_machine_case, tmp_path):
    case = two_machine_case
    upstream = tmp_path / "upstream"
    import subprocess
    subprocess.run(["git", "init", "-b", "main", str(upstream)], check=True, capture_output=True)
    git(upstream, "config", "user.name", "SM Test")
    git(upstream, "config", "user.email", "sm@example.invalid")
    (upstream / "SKILL.md").write_text("Upstream one")
    git(upstream, "add", ".")
    git(upstream, "commit", "-m", "One")
    entry = {"id": "external", "kind": "skill", "origin": "upstream", "scope": "user", "project": None,
             "payload": ".", "upstream": {"git": str(upstream)}}
    publish_catalog(case.repo, {"projects": {}, "resources": [entry]})
    home = case.homes[0]
    state = case.state(home)
    synchronize(case.repo, home, state)
    (upstream / "SKILL.md").write_text("Upstream two")
    git(upstream, "commit", "-am", "Two")
    record_event("codex", {"resource_id": "external", "hook_event_name": "Stop"}, state.path.parent)
    runner = Mock()
    assert cycle(case.repo, home, state, runner).ok
    assert (home / ".agents/skills/external/SKILL.md").read_text() == "Upstream two"
    runner.run.assert_not_called()
    assert not case.mrs


@pytest.mark.parametrize("issue", ["Two redundant steps after one use", "Applicable task missed the trigger"])
def test_record_digest_candidate_and_draft_mr_do_not_change_live_skill(two_machine_case, issue):
    case = two_machine_case
    home = case.homes[0]
    state = case.state(home)
    synchronize(case.repo, home, state)
    before = (home / RELATIVE).read_text()
    event = {"resource_id": "review", "session_id": "single-use", "hook_event_name": "PostToolUse",
             "tool_input": {"command": "Read configuration twice; resolve entry twice"}, "tool_output": "unchanged"}
    if issue.startswith("Applicable"):
        event = {"session_id": "single-use", "hook_event_name": "UserPromptSubmit", "cwd": str(home / "repo/app"),
                 "prompt": "Review this source diff", "available_resource": "review"}
    record_event("codex", event, state.path.parent)
    assert not case.mrs
    class Runner:
        def run(self, prompt):
            evidence = pending_observations(state)[0].id
            data = json.loads(prompt.rsplit("\n", 1)[-1])
            assert all(item["resource"]["origin"] == "self" for item in data["resources"])
            return {"candidates": [{"resource_id": "review", "issue": issue, "evidence": [evidence],
                    "changes": [{"path": "resources/review/SKILL.md", "content": before.replace("Repeat read configuration.\n", "").replace("Repeat resolve entry point.\n", "")}],
                    "expected_checks": ["Keep result verification"]}]}
    report = cycle(case.repo, home, state, Runner())
    assert report.ok, report.details
    assert len(case.mrs) == 1 and case.mrs[0]["title"].startswith("Draft:")
    assert "未验证" in case.mrs[0]["description"]
    assert ("Review this source diff" if issue.startswith("Applicable") else "Read configuration twice") in case.mrs[0]["description"]
    assert (home / RELATIVE).read_text() == before
    assert state.get("findings")[0]["validation"]["status"] == "partial"
    assert not pending_observations(state)
    runner = Mock()
    assert cycle(case.repo, home, state, runner).ok
    runner.run.assert_not_called()
    assert len(case.mrs) == 1


def test_no_effective_model_change_produces_no_mr(two_machine_case):
    case = two_machine_case
    home = case.homes[0]
    state = case.state(home)
    path = case.repo / "resources/review/SKILL.md"
    path.write_text(path.read_text().replace("Repeat read configuration.\n", "").replace("Repeat resolve entry point.\n", ""))
    git(case.repo, "commit", "-am", "Already improved")
    git(case.repo, "push", "origin", "main")
    synchronize(case.repo, home, state)
    before = (home / RELATIVE).read_text()
    record_event("codex", {"resource_id": "review", "hook_event_name": "Stop"}, state.path.parent)
    class Runner:
        def run(self, prompt):
            return {"candidates": [{"resource_id": "review", "issue": "No actual edit", "evidence": [pending_observations(state)[0].id],
                    "changes": [{"path": "resources/review/SKILL.md", "content": before}]}]}
    report = cycle(case.repo, home, state, Runner())
    assert report.ok, report.details
    assert not case.mrs and not report.proposals
