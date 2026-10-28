import ast
import os
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch


SCRIPT = (
    Path(__file__).resolve().parents[1]
    / "resources/user-hook-codex-hindsight/files/.codex/hooks/hindsight_codex.py"
)


class PluginRootTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.codex = self.home / ".codex/plugins/cache/hindsight/hindsight-memory"
        self.claude = self.home / ".claude/plugins/cache/hindsight/hindsight-memory"
        context = ExitStack()
        self.addCleanup(context.close)
        context.enter_context(patch.dict(os.environ, {}, clear=True))
        context.enter_context(patch.object(Path, "home", return_value=self.home))
        # Resolve paths without importing the plugin or starting its memory service.
        function = next(
            node
            for node in ast.parse(SCRIPT.read_text()).body
            if isinstance(node, ast.FunctionDef) and node.name == "default_plugin_root"
        )
        namespace = {"Path": Path, "os": os, "CODEX_HOME": self.home / ".codex"}
        module = ast.Module(body=[function], type_ignores=[])
        exec(compile(module, str(SCRIPT), "exec"), namespace)
        self.resolve = namespace["default_plugin_root"]

    def test_shared_runtime_is_found_when_codex_cache_is_absent(self):
        plugin = self.claude / "0.7.4"
        (plugin / "scripts/lib").mkdir(parents=True)
        self.assertEqual(self.resolve(), plugin)

    def test_codex_cache_takes_priority_over_shared_runtime(self):
        native = self.codex / "0.7.4"
        (native / "scripts/lib").mkdir(parents=True)
        (self.claude / "0.7.5/scripts/lib").mkdir(parents=True)
        self.assertEqual(self.resolve(), native)

    def test_explicit_plugin_paths_take_priority(self):
        (self.claude / "0.7.4/scripts/lib").mkdir(parents=True)
        for key in ("CLAUDE_PLUGIN_ROOT", "HINDSIGHT_PLUGIN_ROOT"):
            with self.subTest(key=key), patch.dict(
                os.environ, {key: str(self.home / "custom-plugin")}
            ):
                self.assertEqual(self.resolve(), self.home / "custom-plugin")
        with patch.dict(
            os.environ,
            {"CLAUDE_PLUGIN_ROOT": "/claude", "HINDSIGHT_PLUGIN_ROOT": "/hindsight"},
        ):
            self.assertEqual(self.resolve(), Path("/hindsight"))


if __name__ == "__main__":
    unittest.main()
