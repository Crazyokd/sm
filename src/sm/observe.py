"""The record entry point deliberately imports only the standard library."""
import hashlib
import json
import os
import re
import uuid
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path

SECRET_ASSIGNMENT = re.compile(r'''(?i)(\b(?:[a-z0-9]+_)*(?:api[_-]?key|password|client_secret|access_token)\b["']?\s*[:=]\s*)("[^"\n]*"|'[^'\n]*'|[^\s,;}&]+)''')


def redact_assignment(match):
    value = match[2].strip("\"'")
    return match[0] if value.startswith(("$", "<", "[redacted]")) else match[1] + '"[redacted]"'


def redact(value, limit=4000):
    if isinstance(value, dict):
        return {k: "[redacted]" if re.search(r"password|api.?key|authorization|access.token|client.secret", k, re.I)
                else redact(v, limit) for k, v in value.items()}
    if isinstance(value, list):
        return [redact(v, limit) for v in (value[:100] if limit is not None else value)]
    if isinstance(value, str):
        value = re.sub(r"(?i)(bearer|basic)\s+[A-Za-z0-9._~+/=-]+", r"\1 [redacted]", value)
        value = re.sub(r"(?:glpat-|sk-proj-)[A-Za-z0-9_-]{12,}", "[redacted]", value)
        value = SECRET_ASSIGNMENT.sub(redact_assignment, value)
        value = re.sub(r"(https?://)[^/\s@]+:[^/\s@]+@", r"\1[redacted]@", value)
        return value[:limit] if limit is not None else value
    return value


def record_event(agent: str, payload: dict, state_dir: Path) -> None:
    if os.environ.get("SM_INTERNAL_RUN") == "1":
        return
    if agent not in {"codex", "kimi"} or not isinstance(payload, dict):
        raise ValueError("invalid recorder input")
    now = datetime.now(timezone.utc)
    payload = dict(payload)
    for key in ("tool_output", "tool_response"):
        if key in payload:
            payload["result_hash"] = hashlib.sha256(json.dumps(payload.pop(key), sort_keys=True).encode()).hexdigest()
    native_id = payload.get("event_id") or payload.get("tool_call_id") or payload.get("tool_use_id")
    event = payload.get("hook_event_name") or payload.get("event", "unknown")
    identity = f"{agent}:{payload.get('session_id', '')}:{event}:{native_id}" if native_id else str(uuid.uuid4())
    data = {"id": hashlib.sha256(identity.encode()).hexdigest(), "agent": agent,
            "timestamp": now.isoformat(), "payload": redact(payload)}
    active = state_dir / "active.json"
    if active.is_file():
        index = json.loads(active.read_text())
        tool_input = json.dumps(payload.get("tool_input", {}), ensure_ascii=False)
        for owned in index.get("owned", []):
            if owned["path"] in tool_input:
                data["resource_id"] = owned["resource_id"]
                data["resource_content_hash"] = index.get("hashes", {}).get(owned["resource_id"])
                break
    encoded = (json.dumps(data, ensure_ascii=False) + "\n").encode()
    if len(encoded) > 32768:
        data["payload"] = {k: data["payload"][k] for k in ("session_id", "cwd", "hook_event_name", "tool_name", "resource_id") if k in data["payload"]}
        data["payload"]["truncated"] = True
        encoded = (json.dumps(data, ensure_ascii=False) + "\n").encode()
    folder = state_dir / "events"
    folder.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor = os.open(folder / f"{now.date()}.jsonl", os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    try:
        if os.write(descriptor, encoded) != len(encoded):
            raise OSError("incomplete observation append")
    finally:
        os.close(descriptor)


@dataclass
class Observation:
    id: str
    agent: str
    session_id: str
    cwd: str
    event: str
    timestamp: str
    resource_id: str | None = None
    resource_content_hash: str | None = None
    evidence_kind: str = "inferred"
    facts: dict = field(default_factory=dict)
    coverage: str = "partial"


def _table(state):
    with state.connect() as db:
        db.execute("CREATE TABLE IF NOT EXISTS observations (id TEXT PRIMARY KEY, value TEXT NOT NULL, processed INTEGER DEFAULT 0)")


def ingest_events(state_dir: Path, state) -> list[Observation]:
    _table(state)
    resources = {r["id"]: r for r in state.get("resources", [])}
    offsets = state.get("event_offsets", {})
    added = []
    errors = state.get("recording_errors", [])
    for path in sorted((state_dir / "events").glob("*.jsonl")):
        offset = offsets.get(str(path), 0)
        with path.open("rb") as stream:
            stream.seek(offset if offset <= path.stat().st_size else 0)
            while True:
                before = stream.tell()
                line = stream.readline()
                if not line:
                    break
                if not line.endswith(b"\n"):
                    stream.seek(before)
                    break
                try:
                    envelope = json.loads(line)
                    if not isinstance(envelope, dict) or not isinstance(envelope.get("payload"), dict):
                        raise ValueError("event requires an object payload")
                    if any(not isinstance(envelope.get(key), str) for key in ("id", "agent", "timestamp")):
                        raise ValueError("invalid event identity")
                    payload = envelope["payload"]
                    if any(key in payload and not isinstance(payload[key], str)
                           for key in ("session_id", "cwd", "hook_event_name", "event", "resource_id")):
                        raise ValueError("invalid event fields")
                    rid = envelope.get("resource_id") or payload.get("resource_id")
                    if rid and resources.get(rid, {}).get("origin") == "upstream":
                        continue
                    if rid not in resources:
                        rid = None
                    event = payload.get("hook_event_name") or payload.get("event", "unknown")
                    row = Observation(envelope["id"], envelope["agent"], payload.get("session_id", ""),
                                      payload.get("cwd", ""), event, envelope["timestamp"], rid,
                                      envelope.get("resource_content_hash"), "execution" if rid else "inferred",
                                      payload, "observed" if rid else "partial")
                    with state.connect() as db:
                        cursor = db.execute("INSERT OR IGNORE INTO observations(id,value) VALUES (?,?)",
                                            (row.id, json.dumps(asdict(row), ensure_ascii=False)))
                        if cursor.rowcount:
                            added.append(row)
                except (ValueError, KeyError, TypeError) as error:
                    errors.append({"file": path.name, "offset": before, "error": type(error).__name__})
            offsets[str(path)] = stream.tell()
    state.set("event_offsets", offsets)
    state.set("recording_errors", errors[-100:])
    return added


def pending_observations(state, limit=1000) -> list[Observation]:
    _table(state)
    with state.connect() as db:
        rows = db.execute("SELECT value FROM observations WHERE processed=0 ORDER BY rowid LIMIT ?", (limit,)).fetchall()
    return [Observation(**json.loads(row[0])) for row in rows]


def complete_observations(state, ids):
    with state.connect() as db:
        db.executemany("UPDATE observations SET processed=1 WHERE id=?", [(value,) for value in ids])
