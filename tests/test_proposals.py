import json
import subprocess

import pytest

from sm.proposals import ChangeSet, create_proposal
from sm.gitlab import submit_mr
from sm.state import State
from conftest import git, publish_skill


def change(**updates):
    data = dict(resource_ids=["review"], reason="Remove duplicate steps", evidence=["event-1"],
                writes={"resources/review/SKILL.md": "Improved"}, deletions=[],
                validation={"status": "partial", "unverified": ["behavior replay"]})
    data.update(updates)
    return ChangeSet(**data)


@pytest.mark.parametrize("path", ["../AGENTS.md", "/absolute", ".git/config", "resources/other/file"])
def test_changeset_rejects_out_of_bounds_paths(path):
    with pytest.raises(ValueError, match="path"):
        change(writes={path: "outside"})


def test_proposal_is_isolated_and_repeat_reuses_branch(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work)
    state = State(tmp_path / "state/state.db")
    first = create_proposal(work, change(), state)
    second = create_proposal(work, change(), state)
    assert first.directory == second.directory
    assert first.branch == second.branch
    assert (first.directory / "resources/review/SKILL.md").read_text() == "Improved"
    assert (work / "resources/review/SKILL.md").read_text() == "Approved"
    assert git(work, "branch", "--show-current") == "main"


def test_upstream_content_cannot_be_proposed(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work)
    data = json.loads((work / "catalog.json").read_text())
    data["resources"][0].update(origin="upstream", upstream={"git": "https://example.invalid/skill"})
    (work / "catalog.json").write_text(json.dumps(data))
    git(work, "add", ".")
    git(work, "commit", "-m", "Use upstream")
    git(work, "push", "origin", "main")
    with pytest.raises(ValueError, match="upstream"):
        create_proposal(work, change(), State(tmp_path / "state/state.db"))


def test_mr_body_is_structured_and_submission_does_not_apply(repo_pair, tmp_path, monkeypatch):
    work, _ = repo_pair
    publish_skill(work)
    state = State(tmp_path / "state/state.db")
    proposal = create_proposal(work, change(reason="First line\nSecond line"), state)
    import sm.gitlab as module
    monkeypatch.setattr(module, "project_location", lambda _: ("gitlab.example.invalid", "me/sm"))
    original = subprocess.run
    calls = []
    def run(argv, **kwargs):
        if argv[0] != "glab":
            return original(argv, **kwargs)
        calls.append(argv)
        if "--input" in argv:
            from pathlib import Path
            body = json.loads(Path(argv[argv.index("--input") + 1]).read_text())
            assert body["target_branch"] == "main"
            assert "First line\nSecond line" in body["description"]
            assert "未验证" in body["description"]
            output = {"iid": 1, "web_url": "https://gitlab.example.invalid/me/sm/-/merge_requests/1"}
        elif argv[2].endswith("/1"):
            output = {"iid": 1, "web_url": "https://gitlab.example.invalid/me/sm/-/merge_requests/1",
                      "source_branch": proposal.branch, "target_branch": "main"}
        else:
            output = []
        return subprocess.CompletedProcess(argv, 0, json.dumps(output), "")
    monkeypatch.setattr(subprocess, "run", run)
    assert submit_mr(proposal).endswith("/1")
    assert proposal.status == "submitted"
    assert not any("merge" in arg for argv in calls for arg in argv if arg.startswith("--"))
    assert (work / "resources/review/SKILL.md").read_text() == "Approved"


def test_submission_failure_keeps_local_proposal(repo_pair, tmp_path, monkeypatch):
    work, _ = repo_pair
    publish_skill(work)
    proposal = create_proposal(work, change(), State(tmp_path / "state/state.db"))
    import sm.gitlab as module
    monkeypatch.setattr(module, "project_location", lambda _: ("gitlab.example.invalid", "me/sm"))
    def fail(*args, **kwargs):
        raise OSError("offline")
    monkeypatch.setattr(module, "api", fail)
    with pytest.raises(OSError, match="offline"):
        submit_mr(proposal)
    assert proposal.status == "draft"


def test_credentials_cannot_leak_through_mr_evidence_or_reason():
    for updates in ({"reason": "Use glpat-abcdefghijklmnop"}, {"evidence": ['api_key="actual-secret"']}):
        with pytest.raises(ValueError, match="credential"):
            change(**updates)


def test_rejected_catalog_does_not_poison_the_next_attempt(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work)
    state = State(tmp_path / "state/state.db")
    catalog = json.loads((work / "catalog.json").read_text())
    catalog["resources"][0]["scope"] = "system"
    with pytest.raises(ValueError):
        create_proposal(work, change(writes={"catalog.json": json.dumps(catalog)}), state)
    proposal = create_proposal(work, change(), state)
    assert (proposal.directory / "resources/review/SKILL.md").read_text() == "Improved"


def test_shared_project_mapping_requires_all_affected_resources_selected(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work, scope="project", project="app")
    folder = work / "resources/other"
    folder.mkdir()
    (folder / "SKILL.md").write_text("Other")
    catalog = json.loads((work / "catalog.json").read_text())
    catalog["resources"].append({**catalog["resources"][0], "id": "other", "payload": "resources/other"})
    (work / "catalog.json").write_text(json.dumps(catalog))
    git(work, "add", ".")
    git(work, "commit", "-m", "Shared project")
    git(work, "push", "origin", "main")
    catalog["projects"]["app"] = "~/repo/elsewhere"
    with pytest.raises(ValueError, match="outside"):
        create_proposal(work, change(writes={"catalog.json": json.dumps(catalog)}), State(tmp_path / "state/state.db"))


def test_existing_proposal_updates_after_main_advances(two_machine_case):
    case = two_machine_case
    state = case.state(case.homes[0])
    proposal = create_proposal(case.repo, change(), state)
    submit_mr(proposal)
    (case.repo / "unrelated.txt").write_text("New approved content")
    git(case.repo, "add", ".")
    git(case.repo, "commit", "-m", "Main advanced")
    git(case.repo, "push", "origin", "main")
    updated = create_proposal(case.repo, change(writes={"resources/review/SKILL.md": "Improved again"}), state)
    assert submit_mr(updated) == proposal.mr_url
    assert len(case.mrs) == 1


def test_existing_mr_refreshes_evidence_and_draft_state(two_machine_case):
    case = two_machine_case
    state = case.state(case.homes[0])
    submit_mr(create_proposal(case.repo, change(validation={"status": "passed"}), state))
    updated = create_proposal(case.repo, change(evidence=["new evidence"], validation={"status": "partial"}), state)
    submit_mr(updated)
    assert "new evidence" in case.mrs[0]["description"]
    assert case.mrs[0]["title"].startswith("Draft:")


def test_rename_preserves_binary_assets_and_only_changes_proposal(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work)
    (work / "resources/review/asset.bin").write_bytes(b"\0\xffpayload")
    git(work, "add", ".")
    git(work, "commit", "-m", "Binary companion")
    git(work, "push", "origin", "main")
    catalog = json.loads((work / "catalog.json").read_text())
    catalog["resources"][0].update(id="renamed", payload="resources/renamed")
    proposal = create_proposal(work, change(resource_ids=["review", "renamed"],
        writes={"catalog.json": json.dumps(catalog)}, renames={"review": "renamed"}), State(tmp_path / "state/state.db"))
    assert (proposal.directory / "resources/renamed/asset.bin").read_bytes() == b"\0\xffpayload"
    assert not (proposal.directory / "resources/review").exists()
    assert (work / "resources/review/asset.bin").exists()
