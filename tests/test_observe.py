import json
import time

from sm.observe import record_event, ingest_events, pending_observations
from sm.state import State


def test_collector_configs_use_lightweight_record_command(tmp_path):
    import shlex
    from sm.providers import codex, kimi
    for provider, agent in ((codex, "codex"), (kimi, "kimi")):
        write = provider.collector(tmp_path, tmp_path / "bin/sm", tmp_path / "state with spaces")
        hooks = write.content["hooks"]
        entry = hooks["PostToolUse"][0]["hooks"][0] if agent == "codex" else next(h for h in hooks if h["event"] == "PostToolUse")
        assert shlex.split(entry["command"])[1:] == ["record", "--agent", agent, "--state-dir", str(tmp_path / "state with spaces")]
        assert entry["timeout"] == 2


def test_internal_analysis_does_not_record_itself(tmp_path, monkeypatch):
    monkeypatch.setenv("SM_INTERNAL_RUN", "1")
    record_event("codex", {"session_id": "internal", "hook_event_name": "Stop"}, tmp_path)
    assert list(tmp_path.rglob("*.jsonl")) == []


def test_native_event_duplicates_do_not_inflate_usage(tmp_path):
    state = State(tmp_path / "state.db")
    state.set("resources", [{"id": "review", "origin": "self"}])
    payload = {"session_id": "one", "hook_event_name": "PostToolUse", "tool_call_id": "call-1",
               "cwd": "/repo/app", "resource_id": "review", "tool_name": "Read",
               "tool_input": {"path": "SKILL.md"}}
    record_event("codex", payload, tmp_path)
    record_event("codex", payload, tmp_path)
    assert len(ingest_events(tmp_path, state)) == 1
    assert ingest_events(tmp_path, state) == []
    assert len(pending_observations(state)) == 1


def test_missing_native_id_does_not_erase_legitimate_repeat(tmp_path):
    state = State(tmp_path / "state.db")
    payload = {"session_id": "one", "hook_event_name": "PostToolUse", "tool_input": {"command": "cat config"}}
    record_event("kimi", payload, tmp_path)
    record_event("kimi", payload, tmp_path)
    assert len(ingest_events(tmp_path, state)) == 2


def test_upstream_is_excluded_and_available_list_is_not_usage(tmp_path):
    state = State(tmp_path / "state.db")
    state.set("resources", [{"id": "external", "origin": "upstream"}, {"id": "review", "origin": "self"}])
    record_event("kimi", {"session_id": "one", "event": "available", "available_skills": ["review"]}, tmp_path)
    record_event("kimi", {"session_id": "one", "resource_id": "external", "event": "used"}, tmp_path)
    rows = ingest_events(tmp_path, state)
    assert len(rows) == 1
    assert rows[0].resource_id is None
    assert rows[0].coverage == "partial"


def test_sensitive_data_and_full_tool_output_not_recorded(tmp_path):
    record_event("codex", {"session_id": "one", "tool_input": {"api_key": "secret-value"},
                           "tool_output": {"password": "another-secret"}}, tmp_path)
    text = next(tmp_path.rglob("*.jsonl")).read_text()
    assert "secret-value" not in text and "another-secret" not in text
    assert "result_hash" in text


def test_corrupt_or_partial_record_is_reported_without_losing_valid_events(tmp_path):
    state = State(tmp_path / "state.db")
    record_event("codex", {"session_id": "one", "event": "Stop"}, tmp_path)
    path = next(tmp_path.rglob("*.jsonl"))
    with path.open("a") as stream:
        stream.write('bad json\n{"partial":')
    assert len(ingest_events(tmp_path, state)) == 1
    assert state.get("recording_errors")
    assert ingest_events(tmp_path, state) == []
def test_string_credentials_are_redacted_before_recording_and_model_input():
    from sm.observe import redact
    for text in ('API_KEY=dummy-sensitive-value curl example.invalid', 'OPENAI_API_KEY=dummy-sensitive-value', 'password="another-sensitive-value"',
                 'Authorization: Basic abcdefghijklmnop', 'https://user:password-value@example.invalid'):
        safe = redact(text)
        assert not any(secret in safe for secret in ("dummy-sensitive-value", "another-sensitive-value", "abcdefghijklmnop", "password-value"))


def test_wrong_shaped_events_do_not_poison_later_records(tmp_path):
    import json
    from sm.observe import ingest_events, record_event
    from sm.state import State
    state = State(tmp_path / "state.db")
    events = tmp_path / "events"
    events.mkdir()
    (events / "0000.jsonl").write_text(json.dumps({"payload": []}) + "\n" + '[]\n')
    record_event("codex", {"hook_event_name": "Stop"}, tmp_path)
    assert len(ingest_events(tmp_path, state)) == 1
    assert len(state.get("recording_errors")) == 2
    assert ingest_events(tmp_path, state) == []
