import json
import shlex
from pathlib import Path

import tomlkit

from ..catalog import OwnedWrite, Resource, UnsupportedResource
from . import copy_payload, hook_command, payload_files


def collector(home: Path, executable: Path, state_dir: Path) -> OwnedWrite:
    command = shlex.join([str(executable), "record", "--agent", "kimi", "--state-dir", str(state_dir)])
    hooks = [{"event": event, "command": command, "timeout": 2}
             for event in ("SessionStart", "UserPromptSubmit", "PostToolUse", "Stop")]
    return OwnedWrite("@observer/kimi", home / ".kimi-code/config.toml", "toml_entries", {"hooks": hooks}, [])


def render(resource: Resource, payload: Path, scope_root: Path) -> list[OwnedWrite]:
    if resource.kind == "hook" and resource.scope == "project":
        raise UnsupportedResource("Kimi project hooks have no native project configuration")
    list(payload_files(payload))
    base = scope_root / ".kimi-code"
    if resource.kind == "skill":
        (payload / "SKILL.md").read_text()
        return copy_payload(resource, payload, scope_root / ".agents/skills" / resource.id)
    if resource.kind == "rule":
        target = base / "AGENTS.md" if resource.scope == "user" else scope_root / "AGENTS.md"
        return [OwnedWrite(resource.id, target, "markdown_block", (payload / "AGENTS.md").read_bytes(), [resource.id])]
    if resource.kind == "agent":
        return [OwnedWrite(resource.id, base / "agents" / f"{resource.id}.md", "file", (payload / "agent.md").read_bytes(), [])]
    if resource.kind == "mcp":
        content = json.loads((payload / "mcp.json").read_text())
        if set(content) != {"mcpServers"}:
            raise ValueError("MCP payload must contain only mcpServers")
        return [OwnedWrite(resource.id, base / "mcp.json", "json_entries", content, list(content["mcpServers"]))]
    content = tomlkit.parse((payload / "config.toml").read_text()).unwrap()
    if set(content) != {"hooks"} or not isinstance(content["hooks"], list):
        raise ValueError("invalid Kimi hooks payload")
    directory = base / "hooks/sm" / resource.id
    for hook in content["hooks"]:
        if set(hook) - {"event", "matcher", "command", "timeout"}:
            raise ValueError("unknown Kimi hook field")
        hook["command"] = hook_command(hook["command"], directory)
    return copy_payload(resource, payload, directory) + [
        OwnedWrite(resource.id, base / "config.toml", "toml_entries", content, [resource.id])]
