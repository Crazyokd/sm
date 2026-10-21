import fcntl
import json
import os
import shutil
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from dataclasses import asdict, dataclass, field
from pathlib import Path

from .apply import apply_writes, atomic_write, plan_apply
from .catalog import contained, load_catalog
from .observe import complete_observations, ingest_events, pending_observations, redact
from .paths import state_dir
from .providers import codex, kimi
from .repository import fetch_approved, fetch_upstream
from .state import State


@dataclass
class InitReport:
    ok: bool
    details: dict


@dataclass
class CycleReport:
    ok: bool
    proposals: list[dict] = field(default_factory=list)
    details: dict = field(default_factory=dict)


@contextmanager
def job_lock(directory: Path):
    directory.mkdir(parents=True, exist_ok=True)
    with (directory / "job.lock").open("a") as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("SM job already running") from error
        try:
            yield
        finally:
            fcntl.flock(stream, fcntl.LOCK_UN)


def systemctl(*args):
    subprocess.run(["systemctl", "--user", *args], check=True, capture_output=True, text=True, timeout=30)


def initialize(repo: Path, home: Path) -> InitReport:
    state = State(state_dir(home) / "state.sqlite3")
    details = {"url": "http://127.0.0.1:8765", "tools": {name: bool(shutil.which(name)) for name in ("git", "codex", "kimi", "glab")}}
    if not details["tools"]["git"]:
        return InitReport(False, {**details, "error": "Git is required"})
    config = home / ".config/sm/config.json"
    atomic_write(config, (json.dumps({"repo": str(repo.resolve())}, indent=2) + "\n").encode())
    directory = home / ".config/systemd/user"
    substitutions = {"python": json.dumps(str(Path(sys.executable).absolute()), ensure_ascii=False).replace("%", "%%"),
                     "repo": json.dumps(str(repo.resolve()), ensure_ascii=False).replace("%", "%%"),
                     "path": json.dumps("PATH=" + os.environ.get("PATH", os.defpath), ensure_ascii=False).replace("%", "%%"),
                     "environment": json.dumps(str(home / ".config/sm/environment"), ensure_ascii=False).replace("%", "%%")}
    for name in ("sm-web.service", "sm-cycle.service", "sm-cycle.timer"):
        template = (Path(__file__).parent / "systemd" / name).read_text()
        atomic_write(directory / name, template.format(**substitutions).encode())
    try:
        systemctl("daemon-reload")
        systemctl("enable", "sm-web.service", "sm-cycle.timer")
        systemctl("start", "sm-cycle.timer")
        systemctl("restart", "sm-web.service")
        details["services"] = "registered"
        report = InitReport(True, details)
    except (OSError, subprocess.SubprocessError) as error:
        report = InitReport(False, {**details, "error": str(error)})
    state.add_run("init", asdict(report))
    return report


def _apply_source(source: Path, home: Path, state: State):
    from .analysis import content_hash
    catalog = load_catalog(source)
    sources, errors = {}, {}
    with tempfile.TemporaryDirectory(prefix="sm-upstream-") as directory:
        for resource in catalog.resources:
            try:
                sources[resource.id] = source if resource.origin == "self" else fetch_upstream(resource, Path(directory) / resource.id)
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                errors[resource.id] = {"status": "failed", "message": str(error)}
        writes = plan_apply(catalog, sources, home, state)
        state.planning_results.update(errors)
        agents = {agent for r in catalog.resources if r.origin == "self" for agent in r.targets}
        executable = Path(sys.executable).parent / "sm"
        for agent in agents:
            writes.append({"codex": codex, "kimi": kimi}[agent].collector(home, executable, state.path.parent))
        report = apply_writes(writes, state)
        resources = [{k: v for k, v in asdict(r).items() if k != "root"} for r in catalog.resources]
        hashes, contents = {}, {}
        for resource in catalog.resources:
            contents[resource.id] = {}
            if resource.id in sources:
                for payload in set(resource.targets.values()):
                    for path in contained(sources[resource.id], payload).rglob("*"):
                        if path.is_file() and not path.is_symlink() and ".git" not in path.parts:
                            if path.suffix in {".md", ".toml", ".json", ".py", ".sh", ".txt"}:
                                contents[resource.id][str(path.relative_to(sources[resource.id]))] = redact(path.read_text(), limit=None)
            if resource.origin == "self" and report.results.get(resource.id, {}).get("status") == "applied":
                hashes[resource.id] = content_hash(resource)
        state.set("resources", resources)
        state.set("catalog", json.loads((source / "catalog.json").read_text()))
        state.set("resource_contents", contents)
        owned = [{"resource_id": w.resource_id, "path": str(w.path)} for w in state.owned() if w.resource_id in hashes]
        atomic_write(state.path.parent / "active.json", json.dumps({"owned": owned, "hashes": hashes}).encode())
        state.set("collector_status", {agent: "已配置；以收到事件为准" + ("，Codex 需原生信任" if agent == "codex" else "") for agent in agents})
    return catalog, report


def synchronize(repo: Path, home: Path, state: State):
    with job_lock(state.path.parent):
        with tempfile.TemporaryDirectory(prefix="sm-approved-") as directory:
            try:
                source = fetch_approved(repo, Path(directory))
                _, report = _apply_source(source, home, state)
                state.add_run("sync", asdict(report))
                return report
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                state.add_run("sync", {"error": str(error)})
                raise


def cycle(repo: Path, home: Path, state: State, runner=None) -> CycleReport:
    from .analysis import analyze_observations, default_runner, to_changeset
    from .evaluation import evaluate_candidate
    from .gitlab import submit_mr
    from .proposals import NoChanges, create_proposal
    with job_lock(state.path.parent):
        result = CycleReport(True)
        try:
            ingest_events(state.path.parent, state)
            with tempfile.TemporaryDirectory(prefix="sm-cycle-") as directory:
                root = Path(directory)
                source = fetch_approved(repo, root / "approved")
                catalog, applied = _apply_source(source, home, state)
                result.ok = applied.ok
                result.details["application"] = applied.results
                observations = pending_observations(state)
                if observations and any(r.origin == "self" for r in catalog.resources):
                    runner = runner or default_runner()
                    candidates = analyze_observations(catalog, observations, runner)
                    findings = []
                    for index, candidate in enumerate(candidates):
                        proposed = root / f"candidate-{index}"
                        shutil.copytree(source, proposed)
                        for path, text in candidate.changes.items():
                            target = contained(proposed, path)
                            target.parent.mkdir(parents=True, exist_ok=True)
                            target.write_text(text)
                        validation = evaluate_candidate(candidate, source, proposed, runner)
                        findings.append({"resource_id": candidate.resource_id, "issue": candidate.issue,
                                         "evidence": candidate.evidence, "validation": asdict(validation)})
                        if validation.status == "failed":
                            continue
                        try:
                            proposal = create_proposal(repo, to_changeset(candidate, validation), state)
                        except NoChanges:
                            continue
                        url = submit_mr(proposal)
                        result.proposals.append({"key": proposal.key, "url": url})
                    state.set("findings", findings)
                complete_observations(state, [o.id for o in observations])
                result.details["observations_processed"] = len(observations)
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
            result.ok = False
            result.details["error"] = str(error)
        state.add_run("cycle", asdict(result))
        return result
