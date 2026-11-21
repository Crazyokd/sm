"""本地准备与面板只能使用独立运行目录，不能写入安装的 Skill。"""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'

def load(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

class WorkspaceTests(unittest.TestCase):
    def test_prepare_is_local_and_keeps_runtime_outside_skill(self):
        prepare = load('prepare')
        with tempfile.TemporaryDirectory() as directory, mock.patch('subprocess.run') as run:
            root = Path(directory) / 'run'
            prepare.prepare(root)
            run.assert_not_called()
            state = json.loads((root / 'active.json').read_text())
            self.assertEqual(state['monitor_mode'], 'waiting_for_vehicles')
            self.assertEqual(state['targets'], {})
            self.assertIsNone(json.loads((root / 'targets.json').read_text())['wx1']['host'])
            self.assertTrue((root / 'sidebar.py').is_file())
        self.assertFalse((SCRIPTS / 'active.json').exists())
        self.assertFalse((SCRIPTS / 'targets.json').exists())

    def test_prepare_cannot_overwrite_a_recording_or_install_tree(self):
        prepare = load('prepare')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sentinel = root / 'active.json'
            original = '{"monitor_mode":"recording"}'
            sentinel.write_text(original)
            with self.assertRaises(FileExistsError):
                prepare.prepare(root)
            self.assertEqual(sentinel.read_text(), original)
        with self.assertRaises(ValueError):
            prepare.prepare(SCRIPTS / 'runtime')

    def test_panel_reuses_only_its_owned_pane(self):
        panel = load('panel')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'sidebar.py').write_text('pass')
            (root / 'panel.json').write_text('{"pane":"%7"}')
            with mock.patch.object(panel, 'tmux', side_effect=[str(root), '']) as run:
                self.assertEqual(panel.show(root, '%1'), '%7')
                self.assertEqual(run.call_args_list[-1].args[0], 'respawn-pane')
            with mock.patch.object(panel, 'tmux', side_effect=['different-workspace', '%8', '']) as run:
                self.assertEqual(panel.show(root, '%1'), '%8')
                self.assertFalse(any(call.args[0] == 'respawn-pane' for call in run.call_args_list))

if __name__ == '__main__':
    unittest.main()
