import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

KINDS = {"skill", "hook", "rule", "agent", "mcp"}
AGENTS = {"codex", "kimi"}


class UnsupportedResource(ValueError):
    pass


def relative_path(value: str) -> Path:
    path = Path(value)
    if not value or path.is_absolute() or ".." in path.parts:
        raise ValueError(f"invalid content path: {value}")
    return path


def contained(root: Path, value: str) -> Path:
    path = root / relative_path(value)
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"path escapes source: {value}")
    return path


@dataclass
class Resource:
    id: str
    kind: str
    origin: str
    scope: str
    project: str | None
    agents: list[str] = field(default_factory=list)
    payloads: dict[str, str] = field(default_factory=dict)
    payload: str | None = None
    upstream: dict[str, Any] | None = None
    root: Path = field(default=Path("."), init=False, repr=False, compare=False)

    def __post_init__(self):
        if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", self.id):
            raise ValueError("invalid resource id")
        if self.kind not in KINDS or self.origin not in {"self", "upstream"}:
            raise ValueError("invalid kind or origin")
        if self.scope not in {"user", "project"}:
            raise ValueError("invalid scope")
        if self.scope == "project" and not self.project:
            raise ValueError("project scope requires project")
        if self.scope == "user" and self.project is not None:
            raise ValueError("user scope cannot declare project")
        if self.kind == "skill":
            if self.agents or self.payloads or not self.payload:
                raise ValueError("Skill requires one payload without client classification")
        else:
            if self.payload is not None or not self.agents or set(self.agents) - AGENTS or len(set(self.agents)) != len(self.agents):
                raise ValueError("invalid agents or payload")
            if set(self.payloads) != set(self.agents):
                raise ValueError("payloads must match agents")
        for path in self.targets.values():
            relative_path(path)
            if self.origin == "self" and Path(path).parts[:1] != ("resources",):
                raise ValueError("self payload path must be under resources/")
        if self.origin == "upstream":
            if not self.upstream or not (self.upstream.get("git") or self.upstream.get("argv")):
                raise ValueError("upstream source requires git or argv")
        elif self.upstream is not None:
            raise ValueError("self resource cannot declare upstream")

    @property
    def targets(self) -> dict[str, str]:
        return dict.fromkeys(sorted(AGENTS), self.payload) if self.kind == "skill" else self.payloads


@dataclass
class Catalog:
    projects: dict[str, str]
    resources: list[Resource]


@dataclass
class OwnedWrite:
    resource_id: str
    path: Path
    mode: str
    content: bytes | dict[str, Any]
    selectors: list[str]
    executable: bool = False


def load_catalog(root: Path) -> Catalog:
    data = json.loads((root / "catalog.json").read_text())
    if set(data) != {"projects", "resources"} or not isinstance(data["projects"], dict):
        raise ValueError("catalog requires projects and resources")
    resources = []
    for item in data["resources"]:
        resource = Resource(**item)
        resource.root = root
        if resource.scope == "project" and resource.project not in data["projects"]:
            raise ValueError(f"unknown project: {resource.project}")
        if resource.origin == "self":
            for path in set(resource.targets.values()):
                target = contained(root, path)
                if not target.is_dir():
                    raise ValueError(f"missing payload: {path}")
        resources.append(resource)
    ids = [r.id for r in resources]
    if len(ids) != len(set(ids)):
        raise ValueError("duplicate resource id")
    for name, path in data["projects"].items():
        if not isinstance(name, str) or not isinstance(path, str) or ".." in Path(path).parts:
            raise ValueError("invalid project directory")
        if not (path.startswith("~/") or Path(path).is_absolute()):
            raise ValueError("project directory must be ~/relative or absolute")
    return Catalog(data["projects"], resources)
