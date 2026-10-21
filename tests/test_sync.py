from pathlib import Path

from sm.catalog import Resource
from sm.repository import fetch_approved, fetch_upstream
from conftest import git, publish_skill


def test_only_remote_main_is_exported(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work)
    git(work, "switch", "-c", "proposal")
    (work / "resources/review/SKILL.md").write_text("Unreviewed")
    git(work, "add", ".")
    git(work, "commit", "-m", "Proposal")
    (work / "resources/review/SKILL.md").write_text("Dirty")
    source = fetch_approved(work, tmp_path / "approved")
    assert (source / "resources/review/SKILL.md").read_text() == "Approved"
    assert (work / "resources/review/SKILL.md").read_text() == "Dirty"
    assert git(work, "branch", "--show-current") == "proposal"


def test_upstream_can_fetch_skill_at_repository_root(repo_pair, tmp_path):
    work, remote = repo_pair
    (work / "SKILL.md").write_text("Upstream skill")
    git(work, "add", ".")
    git(work, "commit", "-m", "Upstream")
    git(work, "push", "origin", "main")
    item = Resource(id="external", kind="skill", origin="upstream", scope="user", project=None,
                    payload=".", upstream={"git": str(remote)})
    source = fetch_upstream(item, tmp_path / "upstream")
    assert (source / "SKILL.md").read_text() == "Upstream skill"
