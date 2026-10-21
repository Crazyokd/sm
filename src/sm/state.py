import base64
import json
import sqlite3
from dataclasses import dataclass, field
from pathlib import Path

from .catalog import OwnedWrite


@dataclass
class ApplyReport:
    results: dict[str, dict] = field(default_factory=dict)

    @property
    def ok(self):
        return all(r["status"] not in {"failed", "conflict"} for r in self.results.values())


class State:
    def __init__(self, path: Path):
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)
        self.planning_results = {}
        with self.connect() as db:
            db.execute("CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            db.execute("CREATE TABLE IF NOT EXISTS ownership (resource TEXT, path TEXT, value TEXT, PRIMARY KEY(resource,path))")
        path.chmod(0o600)

    def connect(self):
        return sqlite3.connect(self.path, timeout=5)

    def get(self, key, default=None):
        with self.connect() as db:
            row = db.execute("SELECT value FROM kv WHERE key=?", (key,)).fetchone()
        return json.loads(row[0]) if row else default

    def set(self, key, value):
        with self.connect() as db:
            db.execute("INSERT OR REPLACE INTO kv VALUES (?,?)", (key, json.dumps(value, ensure_ascii=False)))

    def owned(self):
        with self.connect() as db:
            rows = db.execute("SELECT value FROM ownership").fetchall()
        result = []
        for (value,) in rows:
            data = json.loads(value)
            data["path"] = Path(data["path"])
            if data.pop("binary"):
                data["content"] = base64.b64decode(data["content"])
            result.append(OwnedWrite(**data))
        return result

    def replace_owned(self, path, writes):
        with self.connect() as db:
            db.execute("DELETE FROM ownership WHERE path=?", (str(path),))
            for write in writes:
                binary = isinstance(write.content, bytes)
                data = dict(resource_id=write.resource_id, path=str(path), mode=write.mode,
                            content=base64.b64encode(write.content).decode() if binary else write.content,
                            selectors=write.selectors, executable=write.executable, binary=binary)
                db.execute("INSERT INTO ownership VALUES (?,?,?)", (write.resource_id, str(path), json.dumps(data)))

    def add_run(self, operation, result):
        from datetime import datetime, timezone
        runs = self.get("runs", [])
        runs.append({"operation": operation, "time": datetime.now(timezone.utc).isoformat(), "result": result})
        self.set("runs", runs[-100:])
