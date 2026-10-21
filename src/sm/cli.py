import argparse
import json
import sys
from dataclasses import asdict
from pathlib import Path


def main(argv=None):
    parser = argparse.ArgumentParser(prog="sm", description="Local Codex / Kimi resource management")
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("init", "serve", "sync", "cycle", "status", "propose"):
        command = sub.add_parser(name)
        command.add_argument("--repo", type=Path)
        if name == "serve":
            command.add_argument("--port", type=int, default=8765)
        if name == "propose":
            command.add_argument("change", type=Path)
    record = sub.add_parser("record")
    record.add_argument("--agent", choices=("codex", "kimi"), required=True)
    record.add_argument("--state-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.command == "record":
        from .observe import record_event
        try:
            raw = sys.stdin.read(1048577)
            payload = json.loads(raw) if len(raw) <= 1048576 else {"hook_event_name": "CaptureGap", "reason": "payload exceeds recorder limit"}
            record_event(args.agent, payload, args.state_dir)
        except (OSError, ValueError) as error:
            print(f"SM recording unavailable: {type(error).__name__}", file=sys.stderr)
        return 0
    from .paths import state_dir
    from .state import State
    from .service import initialize, synchronize, cycle
    home = Path.home()
    config = home / ".config/sm/config.json"
    saved = json.loads(config.read_text()) if config.is_file() else {}
    repo = args.repo or Path(saved.get("repo", Path(__file__).resolve().parents[2]))
    state = State(state_dir(home) / "state.sqlite3")
    try:
        if args.command == "serve":
            import uvicorn
            from .web import create_app
            uvicorn.run(create_app(repo, home, state), host="127.0.0.1", port=args.port)
            return 0
        if args.command == "init":
            report = initialize(repo, home)
        elif args.command == "sync":
            report = synchronize(repo, home, state)
        elif args.command == "cycle":
            report = cycle(repo, home, state)
        elif args.command == "propose":
            from .proposals import ChangeSet, create_proposal
            from .gitlab import submit_mr
            proposal = create_proposal(repo, ChangeSet(**json.loads(args.change.read_text())), state)
            print(submit_mr(proposal))
            return 0
        else:
            print(json.dumps({"resources": state.get("resources", []), "results": state.get("resource_results", {}),
                              "runs": state.get("runs", [])[-5:]}, ensure_ascii=False, indent=2))
            return 0
        print(json.dumps(asdict(report), ensure_ascii=False, indent=2))
        return 0 if report.ok else 1
    except (OSError, ValueError, RuntimeError) as error:
        print(f"SM: {error}", file=sys.stderr)
        return 1
