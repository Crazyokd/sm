import json

import pytest
from fastapi.testclient import TestClient

from sm.web import create_app
from sm.service import synchronize
from sm.state import State
from conftest import publish_skill


@pytest.fixture
def client(repo_pair, tmp_path):
    work, _ = repo_pair
    publish_skill(work, "---\nname: review\ndescription: Code review\n---\n# Review\n<script>alert('xss')</script>\n")
    home = tmp_path / "home"
    state = State(tmp_path / "state/state.db")
    synchronize(work, home, state)
    app = create_app(work, home, state)
    with TestClient(app, base_url="http://127.0.0.1:8765") as browser:
        app.state.initial_sync.result(timeout=10)
        yield browser, state, work, home


def test_resource_views_and_filters_are_readable(client):
    browser, _, _, _ = client
    page = browser.get("/resources")
    assert page.status_code == 200
    assert "资源目录" in page.text and "review" in page.text
    assert 'href="/projects"' in page.text
    assert "review" not in browser.get("/resources?origin=upstream").text.split('<tbody>')[1].split('</tbody>')[0]
    for path in ("/projects", "/activity", "/jobs"):
        assert browser.get(path).status_code == 200


def test_detail_escapes_html_and_does_not_show_native_credentials(client):
    browser, _, _, home = client
    (home / ".codex/private.txt").write_text("NATIVE_SECRET_VALUE")
    response = browser.get("/resources/review")
    assert "&lt;script&gt;" in response.text
    assert "<script>alert('xss')</script>" not in response.text
    assert "NATIVE_SECRET_VALUE" not in response.text
    assert "发起修改" in response.text
    assert browser.get("/resources/does-not-exist").status_code == 404


def test_foreign_origin_and_host_cannot_mutate(client):
    browser, _, _, _ = client
    assert browser.post("/proposals", json={}, headers={"Origin":"https://other.invalid"}).status_code == 403
    assert browser.get("/resources", headers={"Host":"other.invalid"}).status_code == 403
    assert browser.post("/proposals", json={}, headers={"Origin":"http://127.0.0.1:8765"}).status_code == 403


def test_ui_edit_creates_a_proposal_and_keeps_live_content(client, monkeypatch):
    browser, state, _, home = client
    import sm.web as module
    monkeypatch.setattr(module, "submit_mr", lambda p: "https://gitlab.example.invalid/mr/1")
    token = browser.app.state.csrf_token
    before = (home / ".agents/skills/review/SKILL.md").read_bytes()
    payload = {"resource_ids":["review"],"reason":"UI edit","evidence":[],
               "writes":{"resources/review/SKILL.md":"Updated only in proposal"},"deletions":[],
               "validation":{"status":"partial","unverified":["manual review"]}}
    response = browser.post("/proposals", json=payload,
                            headers={"Origin":"http://127.0.0.1:8765","X-SM-CSRF":token})
    assert response.status_code == 200, response.text
    assert (home / ".agents/skills/review/SKILL.md").read_bytes() == before
    assert len(state.get("proposals")) == 1


def test_invalid_body_and_path_never_escape_to_filesystem(client):
    browser, _, _, _ = client
    headers={"Origin":"http://127.0.0.1:8765","X-SM-CSRF":browser.app.state.csrf_token}
    payload={"resource_ids":["review"],"reason":"invalid","evidence":[],
             "writes":{"../outside":"bad"},"deletions":[],"validation":{}}
    assert browser.post("/proposals",json=payload,headers=headers).status_code == 400


def test_no_usage_evidence_is_unknown_not_retirement_advice(client):
    browser, _, _, _ = client
    text = browser.get("/activity").text
    assert "尚无可用" in text or "未观测" in text
    assert "建议淘汰" not in text


def test_universal_skill_is_visible_for_either_client(client):
    browser, _, _, _ = client
    for agent in ("codex", "kimi"):
        table = browser.get(f"/resources?agent={agent}").text.split('<tbody>')[1].split('</tbody>')[0]
        assert "review" in table and "通用 Skill" in table


def test_all_resource_types_statuses_and_project_group_are_explicit(client):
    browser, state, _, _ = client
    rows = state.get("resources")
    statuses = ["unsupported", "needs_agent_trust", "failed", "unknown"]
    for kind, status in zip(("hook", "rule", "agent", "mcp"), statuses):
        rows.append({"id": kind, "kind": kind, "origin": "self", "scope": "project",
                     "project": "app", "agents": ["codex"], "payloads": {"codex": f"resources/{kind}"}})
    state.set("resources", rows)
    state.set("resource_results", {r["id"]: {"status": status} for r, status in zip(rows[1:], statuses)})
    page = browser.get("/resources").text
    for label in ("Skill", "Hook", "规则", "Agent 定义", "MCP 配置", "暂不支持", "需原生信任", "应用失败", "尚未应用"):
        assert label in page
    table = browser.get("/resources?scope=project&kind=hook").text.split('<tbody>')[1].split('</tbody>')[0]
    assert "hook" in table and "review" not in table
    groups = browser.get("/projects").text
    assert "用户级" in groups and "app" in groups


def test_improvement_links_to_its_evidence_and_related_mr(client):
    browser, state, _, _ = client
    state.set("findings", [{"resource_id": "review", "issue": "Repeated unchanged reads", "evidence": ["event-one"],
                           "validation": {"status": "partial", "checks": [{"name": "required", "before": True, "after": True}], "unverified": ["Behavior not replayed"]}}])
    state.set("proposals", {"one": {"change": {"resource_ids": ["review"]}, "mr_url": "https://git.example/mr/1"}})
    page = browser.get("/activity").text
    assert 'href="#event-event-one"' in page
    assert 'href="https://git.example/mr/1"' in page
    assert "原有" in page and "修改后" in page


def test_usage_summary_counts_observations_and_sessions_without_claiming_invocations(client):
    from sm.observe import record_event, ingest_events
    browser, state, _, _ = client
    for _ in range(2):
        record_event("codex", {"resource_id": "review", "session_id": "one-session", "hook_event_name": "PostToolUse"}, state.path.parent)
    ingest_events(state.path.parent, state)
    page = browser.get("/activity").text
    assert "观测事件" in page and "涉及会话" in page
    assert "2 条" in page and "1 个" in page
