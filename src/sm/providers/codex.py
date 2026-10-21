import json
import shlex
from pathlib import Path

import tomlkit

from ..catalog import OwnedWrite, Resource
from . import copy_payload, hook_command, payload_files


def collector(home: Path, executable: Path, state_dir: Path) -> OwnedWrite:
    command = shlex.join([str(executable), "record", "--agent", "codex", "--state-dir", str(state_dir)])
    hooks = {event: [{"hooks": [{"type": "command", "command": command, "timeout": 2}]}]
             for event in ("SessionStart", "UserPromptSubmit", "PostToolUse", "Stop")}
    return OwnedWrite("@observer/codex", home / ".codex/hooks.json", "json_entries", {"hooks": hooks}, [])


def render(resource: Resource, payload: Path, scope_root: Path) -> list[OwnedWrite]:
    list(payload_files(payload))
    base = scope_root / ".codex"
    if resource.kind == "skill":
        (payload / "SKILL.md").read_text()
        return copy_payload(resource, payload, scope_root / ".agents/skills" / resource.id)
    if resource.kind == "rule":
        target = base / "AGENTS.md" if resource.scope == "user" else scope_root / "AGENTS.md"
        return [OwnedWrite(resource.id, target, "markdown_block", (payload / "AGENTS.md").read_bytes(), [resource.id])]
    if resource.kind == "agent":
        content = (payload / "agent.toml").read_bytes()
        tomlkit.parse(content.decode())
        return [OwnedWrite(resource.id, base / "agents" / f"{resource.id}.toml", "file", content, [])]
    if resource.kind == "mcp":
        content = tomlkit.parse((payload / "config.toml").read_text()).unwrap()
        if set(content) != {"mcp_servers"}:
            raise ValueError("MCP payload must contain only mcp_servers")
        return [OwnedWrite(resource.id, base / "config.toml", "toml_entries", content, list(content["mcp_servers"]))]
    content = json.loads((payload / "hooks.json").read_text())
    if set(content) != {"hooks"} or not isinstance(content["hooks"], dict):
        raise ValueError("invalid Codex hooks payload")
    directory = base / "hooks/sm" / resource.id
    for groups in content["hooks"].values():
        for group in groups:
            for hook in group["hooks"]:
                if hook.get("type") != "command":
                    raise ValueError("SM hook payload requires command handlers")
                hook["command"] = hook_command(hook["command"], directory)
    writes = copy_payload(resource, payload, directory)
    return writes + [OwnedWrite(resource.id, base / "hooks.json", "json_entries", content, list(content["hooks"]))]
