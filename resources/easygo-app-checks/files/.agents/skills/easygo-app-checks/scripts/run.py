"""Run App checks in the existing development container with its required resources."""
import argparse
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("worktree", type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    app = args.worktree.resolve()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or command[0] not in {"cargo", "just", "rustfmt", "rustc"}:
        parser.error("expected cargo, just, rustfmt, or rustc and its arguments")
    sdk = Path(os.environ.get("EASYGO_ROS_INTERFACES_DIR", str(app / "ros2_interfaces"))).resolve()
    if not (app / "Cargo.toml").is_file() or not (sdk / "install/setup.bash").is_file():
        parser.error("App workspace or installed ROS interface SDK missing: " + str(app))
    top = Path(subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"], cwd=app, text=True
    ).strip()).resolve()
    if top != app:
        parser.error("expected the App Git repository root, not a copy inside another repository: " + str(app))
    common = Path(subprocess.check_output(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], cwd=app, text=True
    ).strip())
    music = Path(os.environ.get(
        "EASYGO_MUSIC_MANIFEST", str(common.parent.parent / "easygo-music-resources/manifest.system.yaml")
    )).resolve()
    if not music.is_file():
        parser.error("music manifest missing; set EASYGO_MUSIC_MANIFEST explicitly: " + str(music))
    container_env = [
        "--env", f"EASYGO_ROS_INTERFACES_DIR={sdk}",
        "--env", f"CARGO_TARGET_DIR={os.environ.get('CARGO_TARGET_DIR', str(app / 'target'))}",
        "--env", f"ROS_LOG_DIR={os.environ.get('ROS_LOG_DIR', str(app / '.cache/ros-logs'))}",
    ]
    for name in (
        "EASYGO_PARAM_DEFINITIONS_DIR",
        "EASYGO_PARAM_MANAGER_BIN",
        "EASYGO_PARAM_MANAGER_CFG_DIR",
    ):
        if name in os.environ:
            container_env.extend(["--env", f"{name}={os.environ[name]}"])
    setup = '''set -e
task_app=$1
task_music=$2
shift 2
test -r "$task_app/Cargo.toml" || { echo "App worktree not readable inside the container" >&2; exit 1; }
test -r "$EASYGO_ROS_INTERFACES_DIR/install/setup.bash" || { echo "Selected ROS interface SDK not readable inside the container" >&2; exit 1; }
test -r "$task_music" || { echo "Music manifest not readable inside the container" >&2; exit 1; }
cd "$task_app"
mkdir -p "$ROS_LOG_DIR"
source /opt/ros/jazzy/setup.bash
source "$EASYGO_ROS_INTERFACES_DIR/install/setup.bash"
export EASYGO_MUSIC_MANIFEST="$task_music"
export LD_LIBRARY_PATH="$EASYGO_ROS_INTERFACES_DIR/install/lib:/opt/sros/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export ROS_DOMAIN_ID=191 ROS_LOCALHOST_ONLY=1 NEXTEST_TEST_THREADS=1
test -r "$EASYGO_MUSIC_MANIFEST"
exec "$@"
'''
    return subprocess.run([
        "docker", "exec", *container_env, "easygo-dev-main", "bash", "-lc", setup,
        "easygo-app-checks", str(app), str(music), *command,
    ]).returncode


if __name__ == "__main__":
    raise SystemExit(main())
