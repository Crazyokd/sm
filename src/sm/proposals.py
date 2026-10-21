import hashlib
import json
import re
import tempfile
import subprocess
from dataclasses import asdict, dataclass, field
from pathlib import Path

from .catalog import contained, load_catalog, relative_path
from .observe import redact
from .repository import git
from .state import State


class NoChanges(ValueError):
    pass


def check_credentials(text):
    if redact(text, limit=None) != text:
        raise ValueError("credential value must remain local")


@dataclass
class ChangeSet:
    resource_ids: list[str]
    reason: str
    evidence: list[str]
    writes: dict[str, str]
    deletions: list[str]
    validation: dict
    renames: dict[str, str] = field(default_factory=dict)

    def __post_init__(self):
        if not self.resource_ids or not self.reason.strip() or not (self.writes or self.deletions):
            raise ValueError("change requires resource ids, reason and changes")
        for path in [*self.writes, *self.deletions]:
            parts = relative_path(path).parts
            if path != "catalog.json" and (len(parts) < 3 or parts[0] != "resources" or parts[1] not in self.resource_ids):
                raise ValueError(f"path is outside the selected resources: {path}")
        if set(self.writes) & set(self.deletions):
            raise ValueError("a path cannot be written and deleted")
        for old, new in self.renames.items():
            if old == new or any(not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", rid) or rid not in self.resource_ids for rid in (old, new)):
                raise ValueError("rename requires two selected resource ids")
        for text in [self.reason, *self.evidence, json.dumps(self.validation), *self.writes.values()]:
            check_credentials(text)


@dataclass
class Proposal:
    key: str
    directory: Path
    branch: str
    change: ChangeSet
    status: str = "draft"
    mr_url: str | None = None
    state: State | None = field(default=None, repr=False)

    def save(self):
        values = self.state.get("proposals", {})
        values[self.key] = {"key": self.key, "directory": str(self.directory), "branch": self.branch,
                            "change": asdict(self.change), "status": self.status, "mr_url": self.mr_url}
        self.state.set("proposals", values)

    def description(self):
        evidence = "\n".join(f"- {item}" for item in self.change.evidence) or "- 用户主动修改"
        validation = json.dumps(self.change.validation, ensure_ascii=False, indent=2)
        return (f"## 问题与证据\n\n{self.change.reason}\n\n{evidence}\n\n"
                f"## 修改依据\n\n自建资源：{', '.join(self.change.resource_ids)}\n\n"
                f"## 修改内容\n\n参见 MR 文件差异；原有运行位置未直接修改。\n\n"
                f"## 对比验证与未验证项\n\n```json\n{validation}\n```\n\n"
                f"## 影响范围\n\n资源归属及 Agent 以 catalog.json 为准。\n")


def create_proposal(repo: Path, change: ChangeSet, state: State) -> Proposal:
    key = hashlib.sha256(json.dumps([sorted(change.resource_ids), change.reason], ensure_ascii=False).encode()).hexdigest()[:16]
    previous = state.get("proposals", {}).get(key)
    directory = state.path.parent / "proposals" / key
    branch = f"sm/{key}"
    directory.parent.mkdir(parents=True, exist_ok=True)
    remote = git(repo, "remote", "get-url", "origin")
    if previous and not (directory / ".git").is_dir():
        raise ValueError("saved proposal directory is missing")
    with tempfile.TemporaryDirectory(prefix=".prepare-", dir=directory.parent) as temporary:
        prepared = Path(temporary) / "repo"
        subprocess.run(["git", "clone", "--branch", branch if previous else "main", "--",
                        str(directory) if previous else remote, str(prepared)],
                       capture_output=True, check=True, timeout=120)
        git(prepared, "remote", "set-url", "origin", remote)
        for setting in ("user.name", "user.email"):
            git(prepared, "config", setting, git(repo, "config", setting))
        if previous:
            git(prepared, "fetch", "origin", "main")
            git(prepared, "merge", "--no-edit", "origin/main")
        else:
            git(prepared, "switch", "-c", branch)
        original = load_catalog(prepared)
        origins = {r.id: r.origin for r in original.resources}
        for old, new in change.renames.items():
            if old not in origins or new in origins:
                raise ValueError("rename source is missing or destination already exists")
            source, destination = prepared / "resources" / old, prepared / "resources" / new
            if destination.exists():
                raise ValueError("rename destination already exists")
            if origins[old] == "self":
                source.rename(destination)
        for path in [*change.writes, *change.deletions]:
            if path != "catalog.json" and origins.get(Path(path).parts[1]) == "upstream":
                raise ValueError("upstream content is not a local optimization target")
            contained(prepared, path)
        for path, text in change.writes.items():
            target = contained(prepared, path)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text)
        for path in change.deletions:
            contained(prepared, path).unlink(missing_ok=True)
        updated = load_catalog(prepared)
        before = {r.id: {k: v for k, v in asdict(r).items() if k != "root"} for r in original.resources}
        after = {r.id: {k: v for k, v in asdict(r).items() if k != "root"} for r in updated.resources}
        if any(before.get(rid) != after.get(rid) for rid in set(before) | set(after) if rid not in change.resource_ids):
            raise ValueError("catalog edits affect resources outside the selected ids")
        if any(r.id not in change.resource_ids and r.scope == "project" and
               original.projects.get(r.project) != updated.projects.get(r.project) for r in original.resources):
            raise ValueError("project mapping affects resources outside the selected ids")
        if any(old in after or new not in after for old, new in change.renames.items()):
            raise ValueError("catalog must reflect the resource rename")
        git(prepared, "add", "--all")
        if not git(prepared, "diff", "origin/main", "--name-only"):
            raise NoChanges("proposal has no changes against approved content")
        if git(prepared, "diff", "--cached", "--name-only"):
            git(prepared, "commit", "-m", change.reason.splitlines()[0][:150])
        backup = Path(temporary) / "previous"
        if directory.exists():
            directory.rename(backup)
        try:
            prepared.rename(directory)
        except OSError:
            if backup.exists():
                backup.rename(directory)
            raise
    proposal = Proposal(key, directory, branch, change,
                        previous.get("status", "draft") if previous else "draft",
                        previous.get("mr_url") if previous else None, state)
    proposal.save()
    return proposal
