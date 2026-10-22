import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

MODULE=Path(__file__).with_name('term-image.py')
spec=importlib.util.spec_from_file_location('term_image', MODULE)
preview=importlib.util.module_from_spec(spec)
spec.loader.exec_module(preview)

class HooksTest(unittest.TestCase):
    def test_previews_preserve_existing_and_other_preview_hooks(self):
        with tempfile.TemporaryDirectory() as directory:
            socket=str(Path(directory)/'tmux.sock')
            def run(*args, env=None):
                return subprocess.check_output(['tmux','-S',socket,*args],env=env,text=True).strip()
            run('new-session','-d','-s','preview-check')
            try:
                run('set-hook','-g','client-session-changed[0]','display-message existing')
                run('set-hook','-g','client-session-changed[2147483646]','display-message other-preview')
                with patch.dict(os.environ, {'TMUX':f'{socket},1,0','TMUX_PANE':'%0'}):
                    with preview.renderer_environment() as env:
                        run('set-hook','-g','client-session-changed','display-message our-preview',env=env)
                        hooks=run('show-hooks','-g')
                        self.assertIn('existing', hooks)
                        self.assertIn('other-preview', hooks)
                        self.assertIn('our-preview', hooks)
                        run('set-hook','-gu','client-session-changed',env=env)
                        hooks=run('show-hooks','-g')
                        self.assertIn('existing', hooks)
                        self.assertIn('other-preview', hooks)
                        self.assertNotIn('our-preview', hooks)
                        run('set-hook','-t','%0','window-layout-changed','display-message our-layout',env=env)
                    self.assertNotIn('our-layout',run('show-hooks','-t','%0'))
            finally:
                run('kill-server')

class InheritanceTest(unittest.TestCase):
    def setUp(self):
        self.directory=tempfile.TemporaryDirectory()
        self.socket=str(Path(self.directory.name)/'tmux.sock')
        self.run_tmux('new-session','-d','-s','preview-check')
        self.pane=self.run_tmux('display-message','-p','#{pane_id}')
        self.environment=patch.dict(os.environ, {'TMUX':f'{self.socket},1,0','TMUX_PANE':self.pane})
        self.environment.start()
    def tearDown(self):
        self.run_tmux('kill-server')
        self.environment.stop()
        self.directory.cleanup()
    def run_tmux(self,*args,env=None):
        return subprocess.check_output(['tmux','-S',self.socket,*args],env=env,text=True).strip()
    def test_inherited_global_hook_executes_during_and_after_preview(self):
        marker=Path(self.directory.name)/'marker'
        self.run_tmux('set-hook','-g','session-window-changed[0]',f"run-shell 'echo hit >> {marker}'")
        other=self.run_tmux('new-window','-d','-P','-F','#{window_id}')
        with preview.renderer_environment() as env:
            self.run_tmux('set-hook','-t',self.pane,'session-window-changed','run-shell true',env=env)
            self.run_tmux('select-window','-t',other)
            deadline=time.monotonic()+1
            while not marker.exists() and time.monotonic()<deadline:
                time.sleep(.01)
            self.assertTrue(marker.exists(), 'preview suppressed existing inherited hook')
        self.run_tmux('select-window','-t','0')
        deadline=time.monotonic()+1
        while len(marker.read_text().splitlines())<2 and time.monotonic()<deadline:
            time.sleep(.01)
        self.assertEqual(len(marker.read_text().splitlines()),2)
    def test_cleanup_after_target_pane_disappears(self):
        other=self.run_tmux('split-window','-h','-d','-P','-F','#{pane_id}')
        with patch.dict(os.environ, {'TMUX_PANE':other}):
            with preview.renderer_environment() as env:
                self.run_tmux('set-hook','-t',other,'session-window-changed','display-message owned-preview',env=env)
                self.run_tmux('kill-pane','-t',other)
        hooks=self.run_tmux('show-hooks','-g')+self.run_tmux('show-hooks','-t',self.pane)
        self.assertNotIn('owned-preview',hooks)

if __name__=='__main__':unittest.main()
