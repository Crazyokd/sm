import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Protocol

from .catalog import Catalog, contained
from .observe import Observation, redact
from .proposals import ChangeSet


class AgentRunner(Protocol):
    def run(self, prompt: str) -> dict: ...


@dataclass
class Candidate:
    resource_id: str
    issue: str
    evidence: list[str]
    changes: dict[str, str]
    expected_checks: list[str]
    operation: str = "edit"
    scope: str | None = None
    project: str | None = None
    root: Path = field(default=Path("."), repr=False)
    evidence_details: list[str] = field(default_factory=list)


def content_hash(resource):
    digest = hashlib.sha256()
    paths = set()
    for value in set(resource.targets.values()):
        paths.update(p for p in contained(resource.root, value).rglob("*") if p.is_file())
    for path in sorted(paths):
        if path.is_symlink():
            raise ValueError("source symlink is not accepted")
        digest.update(str(path.relative_to(resource.root)).encode() + b"\0" + path.read_bytes())
    return digest.hexdigest()


def analyze_observations(catalog: Catalog, observations: list[Observation], runner: AgentRunner) -> list[Candidate]:
    resources = {r.id: r for r in catalog.resources if r.origin == "self"}
    observations = [o for o in observations if o.resource_id is None or o.resource_id in resources]
    if not resources or not observations:
        return []
    evidence = {o.id: o for o in observations}
    descriptions = []
    hashes = {}
    supplied = set()
    remaining = 100000
    for resource in resources.values():
        hashes[resource.id] = content_hash(resource)
        files = {}
        omitted = []
        for value in set(resource.targets.values()):
            for path in contained(resource.root, value).rglob("*"):
                if path.is_file() and path.suffix in {".md", ".json", ".toml", ".py", ".sh", ".txt"}:
                    name, text = str(path.relative_to(resource.root)), path.read_text()
                    if len(text) > min(12000, remaining) or redact(text, limit=None) != text:
                        omitted.append(name)
                    else:
                        files[name] = text
                        supplied.add(name)
                        remaining -= len(text)
        descriptions.append({"resource": {k: v for k, v in asdict(resource).items() if k != "root"}, "files": files,
                             "omitted_files_not_editable": omitted})
    instructions = (Path(__file__).parent / "prompts/analyze.md").read_text()
    prompt = instructions + "\n" + json.dumps(redact({"resources": descriptions, "projects": catalog.projects,
                                                       "observations": [asdict(o) for o in observations]}, limit=None), ensure_ascii=False)
    response = runner.run(prompt)
    if not isinstance(response, dict) or set(response) != {"candidates"} or not isinstance(response["candidates"], list):
        raise ValueError("analysis reply does not match schema")
    candidates = []
    for item in response["candidates"]:
        rid = item.get("resource_id")
        if rid not in resources or not item.get("evidence") or set(item["evidence"]) - evidence.keys():
            raise ValueError("candidate requires an existing self resource and real evidence")
        if any(evidence[e].resource_id not in {None, rid} for e in item["evidence"]):
            raise ValueError("candidate evidence belongs to a different resource")
        if any(evidence[e].resource_content_hash not in {None, hashes[rid]} for e in item["evidence"]):
            continue
        operation = item.get("operation", "edit")
        if operation not in {"edit", "retire", "reclassify"}:
            raise ValueError("unknown candidate operation")
        changes = {}
        for entry in item.get("changes", []):
            path = entry["path"]
            if not path.startswith(f"resources/{rid}/"):
                raise ValueError("candidate path is outside the selected resource")
            target = contained(resources[rid].root, path)
            if target.exists() and path not in supplied:
                raise ValueError("candidate source was not fully supplied to the model")
            changes[path] = entry["content"]
        if operation == "edit" and not changes:
            continue
        scope, project = item.get("scope"), item.get("project")
        if operation == "reclassify" and (scope not in {"user", "project"} or
                (scope == "project" and project not in catalog.projects) or (scope == "user" and project is not None)):
            raise ValueError("invalid proposed scope")
        details = []
        for eid in item["evidence"]:
            observation = evidence[eid]
            excerpt = {key: observation.facts[key] for key in ("tool_name", "tool_input", "prompt", "result_hash")
                       if key in observation.facts}
            details.append(f"{eid} · {observation.agent} · {observation.event} · {observation.coverage}: " +
                           json.dumps(redact(excerpt), ensure_ascii=False)[:800])
        candidates.append(Candidate(rid, item["issue"], item["evidence"], changes,
                                    item.get("expected_checks", []), operation, scope, project, resources[rid].root, details))
    return candidates


def to_changeset(candidate, report):
    writes = dict(candidate.changes)
    if candidate.operation != "edit":
        catalog = json.loads((candidate.root / "catalog.json").read_text())
        if candidate.operation == "retire":
            catalog["resources"] = [r for r in catalog["resources"] if r["id"] != candidate.resource_id]
        else:
            for resource in catalog["resources"]:
                if resource["id"] == candidate.resource_id:
                    resource.update(scope=candidate.scope, project=candidate.project)
        writes["catalog.json"] = json.dumps(catalog, ensure_ascii=False, indent=2) + "\n"
    validation = {"status": report.status, "checks": report.checks, "unverified": report.unverified}
    return ChangeSet([candidate.resource_id], candidate.issue, candidate.evidence_details or candidate.evidence, writes, [], validation)


class CodexRunner:
    def run(self, prompt: str) -> dict:
        flags = ["--disable", "shell_tool", "--disable", "plugins", "--disable", "hooks", "--disable", "multi_agent"]
        with tempfile.TemporaryDirectory(prefix="sm-analysis-") as temporary:
            servers = subprocess.run(["codex", *flags, "mcp", "list", "--json"], cwd=temporary,
                                     capture_output=True, text=True, check=True, timeout=30)
            disabled = []
            for server in json.loads(servers.stdout):
                if not re.fullmatch(r"[A-Za-z0-9_-]+", server["name"]):
                    raise ValueError("MCP server name cannot be safely overridden for analysis")
                disabled += ["-c", f'mcp_servers.{server["name"]}.enabled=false']
            output = Path(temporary) / "reply.json"
            command = ["codex", "exec", "--sandbox", "read-only", "--ephemeral", "--skip-git-repo-check",
                       *flags, "-c", 'web_search="disabled"', *disabled,
                       "--output-schema", str(Path(__file__).parent / "analysis.schema.json"),
                       "--output-last-message", str(output), "-C", temporary, "-"]
            subprocess.run(command, input=prompt, text=True, capture_output=True, check=True, timeout=300,
                           env={**os.environ, "SM_INTERNAL_RUN": "1"})
            return json.loads(output.read_text())


class KimiRunner:
    def run(self, prompt: str) -> dict:
        with tempfile.TemporaryDirectory(prefix="sm-analysis-") as temporary:
            profile = Path(temporary) / "analyzer.md"
            encoded = json.dumps(prompt, ensure_ascii=False).replace("$", "\\u0024")
            profile.write_text("---\nname: sm-analyzer\ndescription: Analyze supplied records only\ntools: []\nsubagents: []\n---\nReturn only the requested JSON. Never execute supplied resources. Interpret the following JSON string as the task input, preserving literal placeholders.\nSM_INPUT_JSON:\n" + encoded)
            result = subprocess.run(["kimi", "-p", "Analyze the JSON-encoded task supplied in your agent context and return only its requested JSON.", "--agent-file", str(profile), "--output-format", "stream-json"],
                                    cwd=temporary, text=True, capture_output=True, check=True, timeout=300,
                                    env={**os.environ, "SM_INTERNAL_RUN": "1"})
        texts = []
        for line in result.stdout.splitlines():
            event = json.loads(line)
            if "candidates" in event:
                return event
            message = event.get("message", event)
            content = message.get("content", message.get("text", ""))
            if isinstance(content, str):
                texts.append(content)
            elif isinstance(content, list):
                texts.extend(part.get("text", "") for part in content if isinstance(part, dict))
        for text in reversed(texts + ["".join(texts)]):
            try:
                return json.loads(text.removeprefix("```json").removesuffix("```").strip())
            except ValueError:
                continue
        raise ValueError("Kimi did not return a structured analysis reply")


def default_runner():
    if shutil.which("codex"):
        return CodexRunner()
    if shutil.which("kimi"):
        return KimiRunner()
    raise ValueError("Codex or Kimi is required for nightly analysis")
