#!/usr/bin/env python3
import argparse
import copy
import importlib.util
import subprocess
import sys
from pathlib import Path

REPO_ALIAS = "vda5050_candidate"


def build_deploy_argv(workspace: Path, host: str, mode: str = "full") -> list[str]:
    argv = [
        str(workspace / "script" / "deploy-dev.py"),
        host,
        REPO_ALIAS,
        "--arch",
        "arm64",
    ]
    if mode == "package":
        argv.append("--package-only")
    elif mode == "deploy":
        argv.append("--deploy-only")
    return argv


def check_source(source: Path) -> None:
    subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=source,
        check=True,
        capture_output=True,
        text=True,
    )
    status = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=source,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    if status:
        raise RuntimeError("refusing to deploy a dirty VDA5050 worktree")


def configure_module(module: object, source: Path) -> None:
    config = copy.deepcopy(module.REPOS["vda5050"])
    source_path = str(source / "src")
    config.arm64_base_paths = [source_path]
    config.amd64_base_paths = [source_path]
    module.REPOS[REPO_ALIAS] = config
    module.REPO_SOURCE_DIRS[REPO_ALIAS] = str(source)


def main() -> int:
    parser = argparse.ArgumentParser(description="Deploy VDA5050 from an explicit Git worktree")
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--host", default="sz-2")
    parser.add_argument("--mode", choices=["full", "package", "deploy"], default="full")
    args = parser.parse_args()

    workspace = args.workspace.resolve()
    source = args.source.resolve()
    deploy_script = workspace / "script" / "deploy-dev.py"
    if not deploy_script.is_file():
        raise RuntimeError(f"deploy script not found: {deploy_script}")
    check_source(source)

    spec = importlib.util.spec_from_file_location("easygo_deploy_dev", deploy_script)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load deploy script: {deploy_script}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    configure_module(module, source)
    if module.repo_source_dir(REPO_ALIAS) != source:
        raise RuntimeError("deploy-dev.py did not select the requested worktree")

    sys.argv = build_deploy_argv(workspace, args.host, args.mode)
    module.main()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(2)
