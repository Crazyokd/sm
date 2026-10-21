import json
import shutil
from dataclasses import asdict
from unittest.mock import Mock

import pytest

from sm.analysis import analyze_observations, to_changeset, CodexRunner, KimiRunner
from sm.catalog import load_catalog
from sm.evaluation import evaluate_candidate
from sm.observe import Observation
from conftest import skill_catalog


def fixture(tmp_path):
    original = tmp_path / "original"
    folder = original / "resources/review"
    folder.mkdir(parents=True)
    (original / "catalog.json").write_text(json.dumps(skill_catalog()))
    (folder / "SKILL.md").write_text("---\nname: review\ndescription: Review\n---\nRead config\nRead config again\nVerify result\n")
    (folder / "evals.json").write_text(json.dumps({"required": ["Verify result"], "forbidden": ["Read config again"]}))
    observation = Observation("event-1", "codex", "session-1", "/repo", "PostToolUse", "2026-10-21", "review",
                              facts={"command": "read config twice", "result_hash": "same"}, coverage="observed")
    payload = {"candidates": [{"resource_id": "review", "issue": "Repeated read", "evidence": ["event-1"],
                "changes": [{"path": "resources/review/SKILL.md", "content": "---\nname: review\ndescription: Review\n---\nRead config\nVerify result\n"}],
                "expected_checks": ["Preserve verification"], "operation": "edit", "scope": None, "project": None}]}
    return original, observation, payload


def test_empty_observations_do_not_call_a_model():
    from sm.catalog import Catalog
    runner = Mock()
    assert analyze_observations(Catalog({}, []), [], runner) == []
    runner.run.assert_not_called()


def test_one_use_can_propose_evidence_based_removal(tmp_path):
    original, observation, payload = fixture(tmp_path)
    runner = Mock()
    runner.run.return_value = payload
    candidates = analyze_observations(load_catalog(original), [observation], runner)
    assert len(candidates) == 1
    proposed = tmp_path / "proposed"
    shutil.copytree(original, proposed)
    for path, content in candidates[0].changes.items():
        (proposed / path).write_text(content)
    report = evaluate_candidate(candidates[0], original, proposed, runner)
    assert report.status == "partial"
    assert report.unverified
    assert any(c["name"] == "forbidden:Read config again" and c["after"] for c in report.checks)
    assert to_changeset(candidates[0], report).resource_ids == ["review"]


def test_removing_required_verification_fails_evaluation(tmp_path):
    original, observation, payload = fixture(tmp_path)
    payload["candidates"][0]["changes"][0]["content"] = "---\nname: review\ndescription: Review\n---\nRead config\n"
    runner = Mock(run=Mock(return_value=payload))
    candidate = analyze_observations(load_catalog(original), [observation], runner)[0]
    proposed = tmp_path / "proposed"
    shutil.copytree(original, proposed)
    (proposed / "resources/review/SKILL.md").write_text(candidate.changes["resources/review/SKILL.md"])
    assert evaluate_candidate(candidate, original, proposed, runner).status == "failed"


@pytest.mark.parametrize("mutation", ["no_evidence", "fake_evidence", "outside", "unknown"])
def test_invalid_candidates_are_rejected(tmp_path, mutation):
    original, observation, payload = fixture(tmp_path)
    catalog = load_catalog(original)
    item = payload["candidates"][0]
    if mutation == "no_evidence":
        item["evidence"] = []
    elif mutation == "fake_evidence":
        item["evidence"] = ["not-real"]
    elif mutation == "outside":
        item["changes"][0]["path"] = "../AGENTS.md"
    else:
        item["resource_id"] = "other"
    runner = Mock(run=Mock(return_value=payload))
    with pytest.raises(ValueError):
        analyze_observations(catalog, [observation], runner)


def test_upstream_only_catalog_never_calls_analysis(tmp_path):
    original, observation, payload = fixture(tmp_path)
    catalog = load_catalog(original)
    catalog.resources[0].origin = "upstream"
    runner = Mock(run=Mock(return_value=payload))
    assert analyze_observations(catalog, [observation], runner) == []
    runner.run.assert_not_called()


def test_cli_runners_disable_tools_and_return_structured_reply(tmp_path, monkeypatch):
    import subprocess
    from pathlib import Path
    calls = []
    def run(argv, **kwargs):
        calls.append(argv)
        if argv[0] == "codex" and "mcp" in argv and "list" in argv:
            assert "plugins" in argv and "hooks" in argv
            return subprocess.CompletedProcess(argv, 0, '[{"name":"production"}]', "")
        assert kwargs["env"]["SM_INTERNAL_RUN"] == "1"
        if argv[0] == "codex":
            assert argv[argv.index("--sandbox") + 1] == "read-only"
            assert 'mcp_servers.production.enabled=false' in argv
            assert not any("bypass" in value for value in argv)
            Path(argv[argv.index("--output-last-message") + 1]).write_text('{"candidates":[]}')
            output = ""
        else:
            profile = Path(argv[argv.index("--agent-file") + 1]).read_text()
            assert "tools: []" in profile and "subagents: []" in profile
            output = json.dumps({"type": "assistant", "message": {"content": [{"type":"text", "text":'{"candidates":[]}' }]}})
        return subprocess.CompletedProcess(argv, 0, output, "")
    monkeypatch.setattr(subprocess, "run", run)
    assert CodexRunner().run("Analyze supplied data") == {"candidates": []}
    assert KimiRunner().run("Analyze supplied data") == {"candidates": []}


def test_retirement_is_a_catalog_proposal_not_live_deletion(tmp_path):
    original, observation, payload = fixture(tmp_path)
    item = payload["candidates"][0]
    item.update(operation="retire", changes=[], issue="Replaced by another resource")
    candidate = analyze_observations(load_catalog(original), [observation], Mock(run=Mock(return_value=payload)))[0]
    change = to_changeset(candidate, Mock(status="partial", checks=[], unverified=["manual review"]))
    assert json.loads(change.writes["catalog.json"])["resources"] == []
    assert (original / "resources/review/SKILL.md").exists()


def test_low_frequency_without_usable_evidence_has_no_automatic_candidate(tmp_path):
    original, observation, _ = fixture(tmp_path)
    runner = Mock(run=Mock(return_value={"candidates": []}))
    observation.resource_id = None
    observation.coverage = "partial"
    assert analyze_observations(load_catalog(original), [observation], runner) == []


def test_kimi_large_input_is_not_passed_as_a_command_line_argument(monkeypatch):
    import subprocess
    from pathlib import Path
    prompt = "${KIMI_WORK_DIR}\n" + "evidence " * 20000
    def run(argv, **kwargs):
        assert max(map(len, argv)) < 8192
        profile = Path(argv[argv.index("--agent-file") + 1]).read_text()
        encoded = profile.split("SM_INPUT_JSON:\n", 1)[1]
        assert "${" not in encoded
        assert json.loads(encoded) == prompt
        return subprocess.CompletedProcess(argv, 0, '{"candidates":[]}', "")
    monkeypatch.setattr(subprocess, "run", run)
    assert KimiRunner().run(prompt) == {"candidates": []}


def test_full_supplied_source_is_not_silently_truncated(tmp_path):
    original, observation, _ = fixture(tmp_path)
    (original / "resources/review/SKILL.md").write_text("x" * 8000 + "SOURCE_TAIL")
    runner = Mock(run=Mock(return_value={"candidates": []}))
    analyze_observations(load_catalog(original), [observation], runner)
    assert "SOURCE_TAIL" in runner.run.call_args[0][0]


def test_omitted_source_cannot_be_overwritten_by_model(tmp_path):
    original, observation, payload = fixture(tmp_path)
    (original / "resources/review/SKILL.md").write_text("x" * 20000)
    with pytest.raises(ValueError, match="supplied"):
        analyze_observations(load_catalog(original), [observation], Mock(run=Mock(return_value=payload)))
