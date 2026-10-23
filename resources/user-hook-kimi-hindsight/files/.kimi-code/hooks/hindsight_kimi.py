#!/usr/bin/env python3
"""Kimi Code lifecycle hooks for the local Hindsight memory API.

Adapted from ~/.codex/hooks/hindsight_codex.py. Shares the same memory bank
(hermes-unified) and agent name (codex) so memories are interchangeable
between Codex and Kimi Code.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path
from typing import Any

CODEX_HOME = Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))).expanduser()


def default_plugin_root() -> Path:
    explicit = os.environ.get("HINDSIGHT_PLUGIN_ROOT") or os.environ.get("CLAUDE_PLUGIN_ROOT")
    if explicit:
        return Path(explicit).expanduser()
    root = CODEX_HOME / "plugins" / "cache" / "hindsight" / "hindsight-memory"
    versions = sorted((p for p in root.glob("*") if (p / "scripts" / "lib").is_dir()), reverse=True)
    if versions:
        return versions[0]
    return root / "0.7.4"


PLUGIN_ROOT = default_plugin_root()
PLUGIN_DATA = Path(
    os.environ.get("HINDSIGHT_PLUGIN_DATA")
    or os.environ.get("CLAUDE_PLUGIN_DATA")
    or str(CODEX_HOME / "plugins" / "data" / "hindsight-memory")
).expanduser()
DEFAULT_API_URL = "http://127.0.0.1:8888"
DEFAULT_BANK_ID = "hermes-unified"
DEFAULT_AGENT_NAME = "codex"
DEFAULT_RECALL_TIMEOUT = 10
DEFAULT_RETAIN_TIMEOUT = 20

KIMI_HOME = Path(os.environ.get("KIMI_CODE_HOME", str(Path.home() / ".kimi-code"))).expanduser()
DEBUG = bool(os.environ.get("HINDSIGHT_KIMI_DEBUG"))


def setup_plugin_env() -> None:
    os.environ.setdefault("CLAUDE_PLUGIN_ROOT", str(PLUGIN_ROOT))
    os.environ.setdefault("CLAUDE_PLUGIN_DATA", str(PLUGIN_DATA))
    os.environ.setdefault("HINDSIGHT_API_URL", DEFAULT_API_URL)
    os.environ.setdefault("HINDSIGHT_BANK_ID", DEFAULT_BANK_ID)
    os.environ.setdefault("HINDSIGHT_AGENT_NAME", DEFAULT_AGENT_NAME)
    os.environ.setdefault("HINDSIGHT_REQUEST_TIMEOUT_SECONDS", str(DEFAULT_RECALL_TIMEOUT))
    PLUGIN_DATA.mkdir(parents=True, exist_ok=True)
    scripts_dir = str(PLUGIN_ROOT / "scripts")
    if scripts_dir not in sys.path:
        sys.path.insert(0, scripts_dir)


setup_plugin_env()

from lib.bank import derive_bank_id, ensure_bank_mission  # noqa: E402
from lib.client import HindsightClient  # noqa: E402
from lib.config import debug_log, load_config  # noqa: E402
from lib.content import (  # noqa: E402
    compose_recall_query,
    format_current_time,
    format_memories,
    prepare_retention_transcript,
    slice_last_turns_by_user_boundary,
    strip_memory_tags,
    truncate_recall_query,
)
from lib.daemon import get_api_url  # noqa: E402
from lib.state import write_state  # noqa: E402


def read_hook_input() -> dict[str, Any]:
    try:
        data = json.load(sys.stdin)
        return data if isinstance(data, dict) else {}
    except (json.JSONDecodeError, EOFError):
        return {}


def debug_dump(tag: str, payload: Any) -> None:
    if not DEBUG:
        return
    try:
        log = PLUGIN_DATA / "kimi_hook_debug.jsonl"
        with log.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps({"tag": tag, "at": time.time(), "payload": payload}, ensure_ascii=False) + "\n")
    except OSError:
        pass


def find_wire_path(session_id: str) -> Path | None:
    if not session_id:
        return None
    matches = sorted(
        KIMI_HOME.glob(f"sessions/*/{session_id}/agents/main/wire.jsonl"),
        key=lambda p: p.stat().st_mtime if p.is_file() else 0,
        reverse=True,
    )
    for path in matches:
        if path.is_file():
            return path
    return None


def text_from_wire_content(content: Any) -> str:
    if isinstance(content, str):
        return strip_memory_tags(content).strip()
    if not isinstance(content, list):
        return ""
    parts: list[str] = []
    for block in content:
        if isinstance(block, dict) and block.get("type") == "text":
            text = block.get("text")
            if isinstance(text, str) and text.strip():
                parts.append(strip_memory_tags(text).strip())
    return "\n".join(part for part in parts if part).strip()


def should_skip_message(role: str, text: str) -> bool:
    stripped = text.lstrip()
    if not stripped:
        return True
    if role == "user" and stripped.startswith("# AGENTS.md instructions"):
        return True
    if role == "user" and stripped.startswith("<environment_context>"):
        return True
    if stripped.startswith("<hindsight_memories>"):
        return True
    return False


def read_kimi_transcript(wire_path: Path | None) -> list[dict[str, Any]]:
    if wire_path is None or not wire_path.is_file():
        return []
    messages: list[dict[str, Any]] = []
    try:
        with wire_path.open(encoding="utf-8") as handle:
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    entry = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if entry.get("type") != "context.append_message":
                    continue
                message = entry.get("message")
                if not isinstance(message, dict):
                    continue
                role = message.get("role")
                if role not in {"user", "assistant"}:
                    continue
                text = text_from_wire_content(message.get("content"))
                if should_skip_message(role, text):
                    continue
                messages.append({"role": role, "content": text, "timestamp": entry.get("time", "")})
    except OSError:
        return []
    return messages


def latest_prompt(hook_input: dict[str, Any], messages: list[dict[str, Any]]) -> str:
    for key in ("prompt", "user_prompt", "userPrompt", "message"):
        value = hook_input.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    for msg in reversed(messages):
        if msg.get("role") == "user":
            content = msg.get("content")
            if isinstance(content, str) and content.strip():
                return content.strip()
    return ""


def load_hindsight_config() -> dict[str, Any]:
    config = load_config()
    config["agentName"] = os.environ.get("HINDSIGHT_AGENT_NAME", DEFAULT_AGENT_NAME)
    config["bankId"] = os.environ.get("HINDSIGHT_BANK_ID", DEFAULT_BANK_ID)
    config["hindsightApiUrl"] = os.environ.get("HINDSIGHT_API_URL", DEFAULT_API_URL)
    config["requestTimeoutSeconds"] = int(os.environ.get("HINDSIGHT_REQUEST_TIMEOUT_SECONDS", DEFAULT_RECALL_TIMEOUT))
    config["retainContext"] = os.environ.get("HINDSIGHT_RETAIN_CONTEXT", "kimi-hook")
    config["retainTags"] = ["kimi", "kimi-hook", "{session_id}"]
    return config


def make_client(config: dict[str, Any], *, allow_daemon_start: bool) -> tuple[HindsightClient | None, str | None]:
    def dbg(*args: Any) -> None:
        debug_log(config, *args)

    try:
        api_url = get_api_url(config, debug_fn=dbg, allow_daemon_start=allow_daemon_start)
        return HindsightClient(
            api_url,
            config.get("hindsightApiToken"),
            request_timeout_override=config.get("requestTimeoutSeconds"),
        ), None
    except Exception as exc:  # graceful degradation for hooks
        return None, str(exc)


def filter_by_min_scores(results: list[dict[str, Any]], min_scores: dict[str, Any]) -> list[dict[str, Any]]:
    floors: dict[str, float] = {}
    for field, floor in (min_scores or {}).items():
        try:
            floors[field] = float(floor)
        except (TypeError, ValueError):
            continue
    if not floors:
        return results

    filtered: list[dict[str, Any]] = []
    for result in results:
        scores = result.get("scores") or {}
        keep = True
        for field, floor in floors.items():
            value = scores.get(field)
            if isinstance(value, (int, float)) and value < floor:
                keep = False
                break
        if keep:
            filtered.append(result)
    return filtered


def command_recall(args: argparse.Namespace) -> int:
    hook_input = read_hook_input()
    debug_dump("recall", hook_input)
    config = load_hindsight_config()
    messages = read_kimi_transcript(find_wire_path(str(hook_input.get("session_id") or "")))
    prompt = latest_prompt(hook_input, messages)
    if len(prompt) < 5:
        return 0

    client, error = make_client(config, allow_daemon_start=False)
    if client is None:
        print(f"[Hindsight] Recall skipped: {error}", file=sys.stderr)
        return 0

    bank_id = derive_bank_id(hook_input, config)
    ensure_bank_mission(client, bank_id, config, debug_fn=lambda *a: debug_log(config, *a))

    recall_context_turns = int(config.get("recallContextTurns", 1) or 1)
    recall_max_query_chars = int(config.get("recallMaxQueryChars", 800) or 800)
    if recall_context_turns > 1:
        query = compose_recall_query(prompt, messages, recall_context_turns, config.get("recallRoles"))
    else:
        query = prompt
    query = truncate_recall_query(query, prompt, recall_max_query_chars)

    try:
        response = client.recall(
            bank_id=bank_id,
            query=query,
            max_tokens=int(config.get("recallMaxTokens", 1024) or 1024),
            budget=config.get("recallBudget", "mid"),
            types=config.get("recallTypes"),
            tags=config.get("recallTags") or None,
            tags_match=config.get("recallTagsMatch") if config.get("recallTags") else None,
            tag_groups=config.get("recallTagGroups") or None,
            timeout=DEFAULT_RECALL_TIMEOUT,
        )
    except Exception as exc:
        print(f"[Hindsight] Recall failed: {exc}", file=sys.stderr)
        return 0

    results = filter_by_min_scores(response.get("results", []), config.get("recallMinScores") or {})
    if not results:
        return 0

    context_message = (
        "<hindsight_memories>\n"
        f"{config.get('recallPromptPreamble', '')}\n"
        f"Current time - {format_current_time()}\n\n"
        f"{format_memories(results)}\n"
        "</hindsight_memories>"
    )
    write_state(
        "kimi_last_recall.json",
        {
            "bank_id": bank_id,
            "result_count": len(results),
            "saved_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        },
    )
    if args.dry_run:
        json.dump({"dryRun": True, "context": context_message}, sys.stdout, ensure_ascii=False)
    else:
        # Kimi appends plain stdout text to context for UserPromptSubmit.
        print(context_message)
    return 0


def resolve_templates(values: list[str], template_vars: dict[str, str]) -> list[str]:
    out: list[str] = []
    for value in values:
        resolved = value
        for key, replacement in template_vars.items():
            resolved = resolved.replace(f"{{{key}}}", replacement)
        if ":" in resolved and resolved.split(":", 1)[1] == "":
            continue
        out.append(resolved)
    return out


def command_retain(args: argparse.Namespace) -> int:
    hook_input = read_hook_input()
    debug_dump("retain", hook_input)
    config = load_hindsight_config()
    messages = read_kimi_transcript(find_wire_path(str(hook_input.get("session_id") or "")))
    if not messages:
        return 0

    messages_to_retain = slice_last_turns_by_user_boundary(messages, int(os.environ.get("HINDSIGHT_KIMI_RETAIN_TURNS", "1")))
    transcript, message_count = prepare_retention_transcript(
        messages_to_retain,
        retain_roles=config.get("retainRoles", ["user", "assistant"]),
        retain_full_window=True,
        include_tool_calls=False,
    )
    if not transcript:
        return 0

    session_id = str(hook_input.get("session_id") or "unknown")
    turn_id = str(hook_input.get("turn_id") or len(messages))
    cwd = str(hook_input.get("cwd") or "")
    model = str(hook_input.get("model") or "")
    content = (
        "# Kimi Code hook transcript retained for Hindsight\n"
        + json.dumps(
            {
                "session_id": session_id,
                "turn_id": turn_id,
                "cwd": cwd,
                "model": model,
                "message_count": message_count,
                "retention_policy": "last_kimi_turn",
            },
            ensure_ascii=False,
            sort_keys=True,
        )
        + "\n\n"
        + transcript
    )

    if args.dry_run:
        json.dump(
            {
                "would_retain": True,
                "session_id": session_id,
                "turn_id": turn_id,
                "message_count": message_count,
                "content_chars": len(content),
            },
            sys.stdout,
            ensure_ascii=False,
        )
        return 0

    client, error = make_client(config, allow_daemon_start=False)
    if client is None:
        print(f"[Hindsight] Retain skipped: {error}", file=sys.stderr)
        return 0

    bank_id = derive_bank_id(hook_input, config)
    ensure_bank_mission(client, bank_id, config, debug_fn=lambda *a: debug_log(config, *a))
    timestamp = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    template_vars = {
        "session_id": session_id,
        "bank_id": bank_id,
        "timestamp": timestamp,
        "user_id": os.environ.get("HINDSIGHT_USER_ID", ""),
    }
    tags = resolve_templates(config.get("retainTags", []), template_vars)
    metadata = {
        "retained_at": timestamp,
        "message_count": str(message_count),
        "session_id": session_id,
        "turn_id": turn_id,
        "cwd": cwd,
        "model": model,
        "source": "kimi-hook",
    }

    try:
        client.retain(
            bank_id=bank_id,
            content=content,
            document_id=f"kimi:{session_id}:turn:{turn_id}",
            context=config.get("retainContext", "kimi-hook"),
            metadata=metadata,
            tags=tags or None,
            timeout=DEFAULT_RETAIN_TIMEOUT,
        )
    except Exception as exc:
        print(f"[Hindsight] Retain failed: {exc}", file=sys.stderr)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Kimi Code Hindsight recall/retain hooks")
    parser.add_argument("command", choices=["recall", "retain"])
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if args.command == "recall":
        return command_recall(args)
    if args.command == "retain":
        return command_retain(args)
    return 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"[Hindsight] Kimi hook unexpected error: {exc}", file=sys.stderr)
        raise SystemExit(0)
