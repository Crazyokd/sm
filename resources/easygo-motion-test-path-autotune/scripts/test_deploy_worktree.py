import unittest
from pathlib import Path
import sys
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
from deploy_worktree import build_deploy_argv, configure_module


class DeployWorktreeTest(unittest.TestCase):
    def test_builds_vda5050_arm64_deploy_command(self):
        argv = build_deploy_argv(Path("/workspace/easygo"), "sz-2")

        self.assertEqual(
            argv,
            [
                "/workspace/easygo/script/deploy-dev.py",
                "sz-2",
                "vda5050_candidate",
                "--arch",
                "arm64",
            ],
        )

    def test_configures_version_and_colcon_paths_from_same_worktree(self):
        config = SimpleNamespace(arm64_base_paths=["old"], amd64_base_paths=["old"])
        module = SimpleNamespace(REPO_SOURCE_DIRS={"vda5050": "old"}, REPOS={"vda5050": config})
        source = Path("/workspace/easygo/.cache/worktrees/vda")

        configure_module(module, source)

        candidate = module.REPOS["vda5050_candidate"]
        self.assertEqual(module.REPO_SOURCE_DIRS["vda5050_candidate"], str(source))
        self.assertEqual(candidate.arm64_base_paths, [str(source / "src")])
        self.assertEqual(candidate.amd64_base_paths, [str(source / "src")])
        self.assertEqual(config.arm64_base_paths, ["old"])


if __name__ == "__main__":
    unittest.main()
