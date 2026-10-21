import hmac
import json
import secrets
import subprocess
from concurrent.futures import ThreadPoolExecutor
from contextlib import asynccontextmanager
from pathlib import Path
from urllib.parse import urlsplit

from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse, RedirectResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates
from markdown_it import MarkdownIt

from .gitlab import submit_mr
from .observe import pending_observations, redact
from .proposals import ChangeSet, create_proposal
from .service import job_lock, synchronize

KIND_LABELS = {"skill": "Skill", "hook": "Hook", "rule": "规则", "agent": "Agent 定义", "mcp": "MCP 配置"}
STATUS_LABELS = {"applied": "文件已应用", "needs_agent_trust": "需原生信任", "next_session": "下次会话加载",
                 "unsupported": "暂不支持", "conflict": "存在冲突", "failed": "应用失败", "unknown": "尚未应用"}


def create_app(repo: Path, home: Path, state) -> FastAPI:
    @asynccontextmanager
    async def lifespan(app):
        executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sm-initial-sync")
        app.state.initial_sync = executor.submit(synchronize, repo, home, state)
        yield
        executor.shutdown(wait=True)

    app = FastAPI(docs_url=None, redoc_url=None, lifespan=lifespan)
    app.state.csrf_token = secrets.token_urlsafe(32)
    package = Path(__file__).parent
    templates = Jinja2Templates(directory=package / "templates")
    app.mount("/static", StaticFiles(directory=package / "static"), name="static")
    markdown = MarkdownIt("commonmark", {"html": False})

    @app.middleware("http")
    async def local_boundary(request, call_next):
        host = request.headers.get("host", "")
        if urlsplit("//" + host).hostname not in {"127.0.0.1", "localhost", "::1"}:
            return JSONResponse({"error": "Local access only"}, status_code=403)
        if request.method not in {"GET", "HEAD", "OPTIONS"}:
            if request.headers.get("origin") != f"{request.url.scheme}://{host}":
                return JSONResponse({"error": "Origin rejected"}, status_code=403)
            if not hmac.compare_digest(request.headers.get("x-sm-csrf", ""), app.state.csrf_token):
                return JSONResponse({"error": "CSRF token required"}, status_code=403)
            if int(request.headers.get("content-length", "0")) > 1048576:
                return JSONResponse({"error": "Change is too large"}, status_code=413)
        response = await call_next(request)
        response.headers["X-Content-Type-Options"] = "nosniff"
        response.headers["X-Frame-Options"] = "DENY"
        response.headers["Content-Security-Policy"] = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"
        return response

    def resource_rows():
        results = state.get("resource_results", {})
        rows = []
        for resource in state.get("resources", []):
            row = dict(resource)
            row["kind_label"] = KIND_LABELS[row["kind"]]
            row["client_label"] = "通用 Skill" if row["kind"] == "skill" else " / ".join(row["agents"])
            row["status"] = results.get(row["id"], {}).get("status", "unknown")
            row["status_label"] = STATUS_LABELS.get(row["status"], row["status"])
            row["status_message"] = results.get(row["id"], {}).get("message", "尚未应用")
            rows.append(row)
        return rows

    def page(request, template, **context):
        rows = resource_rows()
        return templates.TemplateResponse(request=request, name=template, context={
            "nav": request.url.path.split("/")[1], "all_resources": rows,
            "catalog": state.get("catalog", {"projects": {}, "resources": []}),
            "csrf_token": app.state.csrf_token, "kind_labels": KIND_LABELS,
            "total": len(rows), "self_count": sum(r["origin"] == "self" for r in rows),
            "project_count": sum(r["scope"] == "project" for r in rows),
            "pending_count": len(state.get("proposals", {})), **context})

    @app.get("/")
    def index():
        return RedirectResponse("/resources")

    @app.get("/resources")
    def resources(request: Request, origin: str = "", kind: str = "", scope: str = "", agent: str = "", q: str = ""):
        rows = [r for r in resource_rows() if (not origin or r["origin"] == origin)
                and (not kind or r["kind"] == kind) and (not scope or r["scope"] == scope)
                and (not agent or r["kind"] == "skill" or agent in r["agents"]) and (not q or q.casefold() in r["id"].casefold())]
        return page(request, "resources.html", resources=rows, filters=dict(origin=origin, kind=kind, scope=scope, agent=agent, q=q))

    @app.get("/resources/{resource_id}")
    def resource(request: Request, resource_id: str):
        row = next((r for r in resource_rows() if r["id"] == resource_id), None)
        if row is None:
            raise HTTPException(404, "Resource not found")
        files = state.get("resource_contents", {}).get(resource_id, {})
        articles = []
        for name, content in files.items():
            body = content
            if name.endswith(".md") and body.startswith("---\n") and len(body.split("---", 2)) == 3:
                body = body.split("---", 2)[2]
            articles.append({"name": name, "html": markdown.render(body) if name.endswith(".md") else None, "text": content})
        proposals = [p for p in state.get("proposals", {}).values() if resource_id in p["change"]["resource_ids"]]
        return page(request, "resource.html", resource=row, files=files, articles=articles, proposals=proposals)

    @app.get("/projects")
    def projects(request: Request):
        groups = {}
        for row in resource_rows():
            groups.setdefault(row["project"] if row["scope"] == "project" else "用户级", []).append(row)
        return page(request, "projects.html", groups=groups)

    @app.get("/activity")
    def activity(request: Request):
        pending_observations(state, limit=1)
        with state.connect() as db:
            records = db.execute("SELECT value FROM observations ORDER BY rowid DESC LIMIT 100").fetchall()
            counts = {rid: {"events": count, "sessions": sessions, "last_seen": last_seen}
                      for rid, count, sessions, last_seen in db.execute(
                          "SELECT json_extract(value,'$.resource_id'), COUNT(*), "
                          "COUNT(DISTINCT NULLIF(json_extract(value,'$.session_id'),'')), "
                          "MAX(json_extract(value,'$.timestamp')) FROM observations GROUP BY 1")}
        events = []
        for (value,) in records:
            row = json.loads(value)
            facts = row["facts"]
            tool_input = facts.get("tool_input", {})
            excerpt = tool_input.get("command", tool_input.get("path", "")) if isinstance(tool_input, dict) else ""
            events.append({k: row[k] for k in ("id", "timestamp", "agent", "resource_id", "event", "coverage")})
            events[-1]["excerpt"] = redact(str(excerpt))[:180]
        findings = state.get("findings", [])
        for finding in findings:
            finding["proposals"] = [p for p in state.get("proposals", {}).values()
                                    if finding["resource_id"] in p["change"]["resource_ids"]]
        usage = [{**r, "usage": counts.get(r["id"])} for r in resource_rows() if r["origin"] == "self"]
        return page(request, "activity.html", events=events, findings=findings, usage=usage,
                    recording_errors=state.get("recording_errors", []))

    @app.get("/jobs")
    def jobs(request: Request):
        runs = list(reversed(state.get("runs", [])))
        return page(request, "jobs.html", runs=runs, collectors=state.get("collector_status", {}))

    @app.post("/proposals")
    async def proposals(request: Request):
        try:
            body = await request.json()
            change = ChangeSet(**body)
            with job_lock(state.path.parent):
                proposal = create_proposal(repo, change, state)
                try:
                    url = submit_mr(proposal)
                    return {"status": "submitted", "key": proposal.key, "url": url, "message": "MR 已提交，审核合并后将在下次同步应用。"}
                except (OSError, ValueError, subprocess.SubprocessError) as error:
                    return JSONResponse({"status": "draft", "key": proposal.key,
                        "message": "本地提案已保存，远端提交未完成：" + redact(str(error))[:500]}, status_code=202)
        except RuntimeError as error:
            return JSONResponse({"error": str(error)}, status_code=409)
        except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
            return JSONResponse({"error": redact(str(error))[:500]}, status_code=400)

    return app
