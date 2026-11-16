"""Data contracts for the reusable, offline two-robot replay helper."""
import csv
import importlib.util
import hashlib
import json
import shutil
import subprocess
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SCRIPT = Path(__file__).resolve().parents[1] / 'resources/simenv-skill-simenv-replay-video/files/.agents/skills/simenv-replay-video/scripts/prepare.py'
HEADER = 'schema,iteration,sim_ns,dt_ns,model,link_entity,valid,px,py,pz,qx,qy,qz,qw,vx,vy,vz,wx,wy,wz,error'.split(',')


class ReplayDataTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        spec = importlib.util.spec_from_file_location('replay_prepare', SCRIPT)
        self.api = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.api)
        (self.root / 'world.sdf').write_text('<sdf><world><model name="walls"><link name="wall"><pose>2 3 0 0 0 0</pose><collision name="c"><geometry><box><size>2 4 1</size></box></geometry></collision></link></model></world></sdf>')
        (self.root / 'robot.sdf').write_text('<sdf><model name="robot"/></sdf>')
        self.result = {'passed': True, 'motion_start_anchor': {'sim_ns': 1000000000}, 'rtf_sim_end': 1006000000, 'terminal_anchors': {'1': {'sim_ns': 1003000000}, '2': {'sim_ns': 1006000000}}, 'physical': {'passed': True, 'single_vehicle': {'robot1': {'minimum_continuous_bound_m': .05, 'maximum_sample_gap_s': .003}, 'robot2': {'minimum_continuous_bound_m': .06, 'maximum_sample_gap_s': .003}}, 'minimum_pair_continuous_bound_m': .1}, 'real_time_factor': .99}
        self.config = {'source_label': 'fixture revision', 'world': {'sdf': 'world.sdf', 'wall_model': 'walls'}, 'body': {'source': 'robot.sdf', 'outline': [[-.4,-.2],[.4,-.2],[.4,.2],[-.4,.2]]}, 'scenes': [{'title': 'custom crossing', 'result': 'result.json', 'physics': ['r1.csv','r2.csv'], 'cooperation': ['r1.jsonl',None], 'goals': [{'x':1.,'y':0.},{'x':2.,'y':0.}]}]}
        for robot in (1, 2):
            with (self.root / f'r{robot}.csv').open('w') as f:
                writer = csv.writer(f); writer.writerow(HEADER)
                for k in range(6):
                    writer.writerow([1, k+1, 994000000+k*3000000, 3000000, f'robot{robot}', robot, 1, k*.00003, robot, 0, 0, 0, 0, 1, .01, 0, 0, 0, 0, 0, ''])
        (self.root / 'r1.jsonl').write_text(json.dumps({'kind':'cooperation','sim_ns':1000000000,'state':1,'components':[{'recommendation':2}]})+'\n')

    def prepare(self):
        (self.root / 'result.json').write_text(json.dumps(self.result))
        (self.root / 'config.json').write_text(json.dumps(self.config))
        return self.api.prepare(self.root / 'config.json')

    def test_clip_native_phase_and_use_terminal_physics_anchors(self):
        m = self.prepare(); s = m['scenes'][0]
        self.assertEqual([round(row[0],3) for row in s['poses']['1']], [0., .003, .006])
        self.assertEqual(s['terminal_s'], {'1': .003, '2': .006})
        self.assertEqual(s['title'], 'custom crossing')

    def test_missing_iteration_is_rejected(self):
        p = self.root / 'r1.csv'
        with p.open() as stream:
            rows = list(csv.reader(stream))
        del rows[4]
        with p.open('w') as f: csv.writer(f).writerows(rows)
        with self.assertRaisesRegex(ValueError, 'gap|missing|连续'): self.prepare()

    def test_physical_success_does_not_relabel_failed_run(self):
        self.result.update(passed=False, real_time_factor=.89)
        self.assertFalse(self.prepare()['scenes'][0]['passed'])

    def test_missing_terminal_anchor_stays_unknown(self):
        del self.result['terminal_anchors']['2']
        self.assertIsNone(self.prepare()['scenes'][0]['terminal_s']['2'])

    def test_advice_is_not_inferred_from_motion(self):
        s = self.prepare()['scenes'][0]
        self.assertGreater(s['poses']['1'][0][4], 0)
        self.assertEqual(self.api.advice_at(s['advice']['1'], .001), (2, 0.))
        self.assertEqual(self.api.advice_at(s['advice']['2'], .001), (None, None))

    def test_completion_is_three_valued(self):
        self.assertIsNone(self.api.completion_at(None, 50.))
        self.assertFalse(self.api.completion_at(10., 9.))
        self.assertTrue(self.api.completion_at(10., 10.))

    def test_world_geometry_uses_selected_model(self):
        walls = self.prepare()['walls']
        self.assertEqual(walls, [[[1.,1.],[3.,1.],[3.,5.],[1.,5.]]])

    def test_source_files_are_not_modified(self):
        before = (self.root / 'r1.csv').read_bytes()
        m = self.prepare()
        self.assertEqual((self.root / 'r1.csv').read_bytes(), before)
        self.assertIn('sha256', m['scenes'][0]['sources']['physics'][0])

    def test_non_boolean_status_is_rejected(self):
        self.result['passed'] = 'true'
        with self.assertRaisesRegex(ValueError, 'passed|bool'): self.prepare()

    def test_window_end_must_be_present_in_both_sources(self):
        self.result['rtf_sim_end'] = 1007000000
        with self.assertRaisesRegex(ValueError, 'window|窗口|end'): self.prepare()


@unittest.skipUnless(importlib.util.find_spec('PIL') and shutil.which('fc-match'), 'render integration requires Pillow and Fontconfig')
class RenderIntegrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, str(SCRIPT.parent))
        spec = importlib.util.spec_from_file_location('replay_render', SCRIPT.with_name('render.py'))
        cls.api = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.api)

    def test_chinese_labels_have_distinct_glyphs(self):
        f = self.api.font(28)
        a, b = f.getmask('回'), f.getmask('放')
        self.assertNotEqual((a.size, hashlib.sha256(bytes(a)).hexdigest()), (b.size, hashlib.sha256(bytes(b)).hexdigest()))

    @unittest.skipUnless(shutil.which('ffmpeg') and shutil.which('ffprobe'), 'decode integration requires ffmpeg and ffprobe')
    def test_corrupt_packet_is_not_certified_as_decoded(self):
        with tempfile.TemporaryDirectory() as directory:
            video = Path(directory) / 'clip.mp4'
            subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i','testsrc2=size=64x64:rate=12','-t','1','-c:v','libx264','-g','12','-bf','0','-pix_fmt','yuv420p',str(video)],check=True)
            self.api.verify_video(video)
            info = json.loads(subprocess.check_output(['ffprobe','-v','error','-select_streams','v','-show_packets','-show_entries','packet=pos,size,flags','-of','json',str(video)],text=True))
            packet = next(p for p in info['packets'] if 'K' not in p['flags'])
            position, size = int(packet['pos']), int(packet['size'])
            blob = bytearray(video.read_bytes())
            blob[position+5:position+size] = b'\xff' * (size-5)
            video.write_bytes(blob)
            with self.assertRaises((subprocess.CalledProcessError, RuntimeError)):
                self.api.verify_video(video)


if __name__ == '__main__':
    unittest.main()
