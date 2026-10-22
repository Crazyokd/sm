#!/usr/bin/env python3
"""Launch the installed current-session forge workflow."""

from __future__ import annotations

import os
import sys
from pathlib import Path


def main() -> int:
    configured = os.environ.get("DEV_KNOWLEDGE_BASE_REPO")
    candidates = [
        Path(configured).expanduser() / "tooling" / "scripts" / "forge_current_session.py"
        if configured
        else None,
        Path.home() / "repo" / "developer-knowledge" / "tooling" / "scripts" / "forge_current_session.py",
        Path.home() / ".hermes" / "scripts" / "forge_current_session.py",
    ]
    for candidate in candidates:
        if candidate and candidate.exists():
            os.execv(sys.executable, [sys.executable, str(candidate), *sys.argv[1:]])
    searched = ", ".join(str(path) for path in candidates if path)
    raise RuntimeError(f"forge_current_session.py not found; searched: {searched}")


if __name__ == "__main__":
    raise SystemExit(main())
