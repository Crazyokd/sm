import json
import subprocess
import sys
from unittest.mock import Mock

import pytest

from sm.service import initialize, synchronize, cycle, job_lock
from sm.state import State
from conftest import publish_skill


def test_cycle_lock_prevents_reentry(tmp_path):
    with job_lock(tmp_path):
        with pytest.raises(RuntimeError, match="already running"):
            with job_lock(tmp_path):
                pass


def test_initialize_creates_idempotent_user_services(repo_pair, tmp_path, monkeypatch):
    work, _ = repo_pair
    home = tmp_path / "home"
    import sm.service as module
    calls = []
    monkeypatch.setattr(module, "systemctl", lambda *args: calls.append(args))
    first = initialize(work, home)
    unit = home / ".config/systemd/user/sm-cycle.timer"
    assert unit.is_file()
    assert str(sys.executable) in (unit.parent / "sm-web.service").read_text()
    assert "02:00:00" in unit.read_text()
    before = unit.read_bytes()
    second = initialize(work, home)
    assert unit.read_bytes() == before
    assert len(list(unit.parent.glob("sm-*.service"))) == 2
    assert first.ok and second.ok
    assert not (home / ".codex/hooks.json").exists()


def test_unavailable_user_service_is_an_explicit_result(repo_pair, tmp_path, monkeypatch):
    work, _ = repo_pair
    import sm.service as module
    def fail(*args):
        raise OSError("no user bus")
    monkeypatch.setattr(module, "systemctl", fail)
    result = initialize(work, tmp_path / "home")
    assert not result.ok
    assert "no user bus" in json.dumps(result.details)


def test_synchronize_applies_main_and_registers_collectors_once(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work)
    home = tmp_path / "home"
    state = State(tmp_path / "state/state.db")
    assert synchronize(work, home, state).ok
    first = (home / ".codex/hooks.json").read_bytes()
    assert (home / ".agents/skills/review/SKILL.md").read_text() == "Approved"
    assert not (home / ".kimi-code/skills/review").exists()
    assert synchronize(work, home, state).ok
    assert (home / ".codex/hooks.json").read_bytes() == first
    assert state.get("resources")[0]["origin"] == "self"


def test_empty_cycle_never_calls_a_model_or_creates_a_proposal(repo_pair, tmp_path):
    work, _ = repo_pair
    runner = Mock()
    report = cycle(work, tmp_path / "home", State(tmp_path / "state/state.db"), runner)
    assert report.ok
    runner.run.assert_not_called()
    assert report.proposals == []


def test_one_invalid_resource_does_not_hide_other_results(repo_pair, tmp_path):
    from conftest import git
    work, _ = repo_pair
    publish_skill(work)
    bad = work / "resources/bad"
    bad.mkdir()
    (bad / "note.txt").write_text("Missing SKILL.md")
    catalog = json.loads((work / "catalog.json").read_text())
    catalog["resources"].append({**catalog["resources"][0], "id": "bad", "payload": "resources/bad"})
    (work / "catalog.json").write_text(json.dumps(catalog))
    git(work, "add", ".")
    git(work, "commit", "-m", "Invalid resource")
    git(work, "push", "origin", "main")
    report = synchronize(work, tmp_path / "home", State(tmp_path / "state/state.db"))
    assert report.results["bad"]["status"] == "failed"
    assert report.results["review"]["status"] == "applied"


def test_generated_units_pass_native_syntax_check(repo_pair, tmp_path, monkeypatch):
    import shutil
    if not shutil.which("systemd-analyze"):
        pytest.skip("systemd-analyze is not installed")
    import sm.service as module
    monkeypatch.setattr(module, "systemctl", lambda *args: None)
    home = tmp_path / "home"
    initialize(repo_pair[0], home)
    units = list((home / ".config/systemd/user").glob("sm-*"))
    result = subprocess.run(["systemd-analyze", "verify", *map(str, units)], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
