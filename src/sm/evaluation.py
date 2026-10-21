import ast
import json
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

import tomlkit

from .catalog import contained


@dataclass
class ValidationReport:
    status: str
    checks: list[dict] = field(default_factory=list)
    unverified: list[str] = field(default_factory=list)


def evaluate_candidate(candidate, original: Path, proposed: Path, runner=None) -> ValidationReport:
    checks = []
    for name in candidate.changes:
        path = contained(proposed, name)
        try:
            text = path.read_text()
            if path.suffix == ".json":
                json.loads(text)
            elif path.suffix == ".toml":
                tomlkit.parse(text)
            elif path.suffix == ".py":
                ast.parse(text)
            elif path.suffix == ".sh":
                subprocess.run(["bash", "-n", str(path)], capture_output=True, check=True, timeout=10)
            elif path.name == "SKILL.md":
                header = text.split("---", 2)
                if len(header) != 3 or not all(f"{key}:" in header[1] for key in ("name", "description")):
                    raise ValueError("Skill requires name and description frontmatter")
            checks.append({"name": f"syntax:{name}", "kind": "static", "after": True})
        except (ValueError, OSError, SyntaxError, subprocess.SubprocessError) as error:
            checks.append({"name": f"syntax:{name}", "kind": "static", "after": False, "error": type(error).__name__})
    casefile = original / "resources" / candidate.resource_id / "evals.json"
    if casefile.is_file():
        cases = json.loads(casefile.read_text())
        before = "\n".join(p.read_text() for p in (original / "resources" / candidate.resource_id).rglob("*.md"))
        after = "\n".join(p.read_text() for p in (proposed / "resources" / candidate.resource_id).rglob("*.md"))
        for word in cases.get("required", []):
            checks.append({"name": f"required:{word}", "kind": "approved_fragment_check", "before": word in before, "after": word in after})
        for word in cases.get("forbidden", []):
            checks.append({"name": f"forbidden:{word}", "kind": "approved_fragment_check", "before": word not in before, "after": word not in after})
    failed = any(not item["after"] for item in checks)
    return ValidationReport("failed" if failed else "partial", checks,
                            ["尚未真实复跑业务行为；静态和文本案例检查不证明执行效果，需人工审阅"])
