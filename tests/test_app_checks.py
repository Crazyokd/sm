import json
import ast
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "resources/easygo-app-checks/files/.agents/skills/easygo-app-checks/scripts/run.py"
MIRROR = ROOT / "resources/easygo-workspace-app-checks/files/.agents/skills/easygo-app-checks/scripts/run.py"


class AppChecksTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.app = self.root / "app"
        subprocess.run(["git", "init", "-q", str(self.app)], check=True)
        (self.app / "Cargo.toml").write_text("[workspace]\n")
        self.sdk = self.root / "selected sdk"
        (self.sdk / "install").mkdir(parents=True)
        (self.sdk / "install/setup.bash").write_text("")
        self.music = self.root / "music.yaml"
        self.music.write_text("[]\n")
        self.capture = self.root / "docker.json"
        tools = self.root / "tools"
        tools.mkdir()
        docker = tools / "docker"
        docker.write_text("#!/usr/bin/env python3\nimport json,os,sys\nfrom pathlib import Path\nPath(os.environ['CHECK_ARGS']).write_text(json.dumps(sys.argv[1:]))\nsys.exit(7)\n")
        docker.chmod(0o700)
        self.env = {**os.environ, "PATH": str(tools) + ":" + os.environ["PATH"], "CHECK_ARGS": str(self.capture), "EASYGO_MUSIC_MANIFEST": str(self.music), "EASYGO_ROS_INTERFACES_DIR": str(self.sdk), "CARGO_TARGET_DIR": str(self.root / "cache target"), "ROS_LOG_DIR": str(self.root / "ros logs"), "EASYGO_PARAM_MANAGER_BIN": "/selected/param manager"}

    def run_script(self, app):
        return subprocess.run(["python3", str(SCRIPT), str(app), "--", "cargo", "test", "literal $(untouched)"], env=self.env, capture_output=True, text=True)

    def test_selected_sdk_cache_and_logs_reach_docker_with_original_arguments(self):
        result = self.run_script(self.app)
        self.assertEqual(result.returncode, 7, result.stderr)
        args = json.loads(self.capture.read_text())
        for name in ("EASYGO_ROS_INTERFACES_DIR", "CARGO_TARGET_DIR", "ROS_LOG_DIR", "EASYGO_PARAM_MANAGER_BIN"):
            self.assertIn(name + "=" + self.env[name], args)
        self.assertEqual(args[-3:], ["cargo", "test", "literal $(untouched)"])
        self.assertEqual(SCRIPT.read_bytes(), MIRROR.read_bytes())

    def test_gitless_copy_inside_parent_repo_is_rejected_before_docker(self):
        copy = self.app / "copy"
        (copy / "ros2_interfaces/install").mkdir(parents=True)
        (copy / "Cargo.toml").write_text("[workspace]\n")
        (copy / "ros2_interfaces/install/setup.bash").write_text("")
        result = self.run_script(copy)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.capture.exists())

    def test_relative_ros_logs_are_created_in_the_app_worktree(self):
        node = next(node for node in ast.walk(ast.parse(SCRIPT.read_text())) if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == "setup" for target in node.targets))
        setup = ast.literal_eval(node.value)
        env = {**self.env, "ROS_LOG_DIR": "relative-logs"}
        result = subprocess.run(["bash", "-c", "source() { :; };\n" + setup, "checks", str(self.app), str(self.music), "bash", "-c", 'test -d "$ROS_LOG_DIR"'], cwd="/proc", env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.app / "relative-logs").is_dir())


if __name__ == "__main__":
    unittest.main()
