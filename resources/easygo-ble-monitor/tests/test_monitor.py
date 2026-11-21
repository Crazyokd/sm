"""离线验证：待机不联网、地址固定、身份校验、原始记录完整性和侧栏数据质量。"""
import ast
import copy
import importlib.util
import io
import signal
import json
import math
from collections import Counter
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest import mock

BASE = Path(__file__).resolve().parents[1] / 'scripts'

def load(name):
    spec = importlib.util.spec_from_file_location(name, BASE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

START = load('start_recording')
VIEW = load('sidebar')
STOP = load('stop_recording')
VALID = {
    'wx1': {'host': '192.0.2.10', 'ssh_user': 'std', 'ssh_port': 22, 'robot_id': 'A', 'bluetooth_mac': '02:00:00:00:00:01'},
    'wx2': {'host': '192.0.2.11', 'ssh_user': 'std', 'ssh_port': 22, 'robot_id': 'B', 'bluetooth_mac': '02:00:00:00:00:02'},
}

class MonitorTests(unittest.TestCase):
    def test_live_zero_reception_is_explicit_before_first_peer(self):
        tree = ast.parse((BASE / 'collector.py').read_text())
        assignment = next(node for node in tree.body if isinstance(node, ast.Assign)
                          and any(isinstance(target, ast.Name) and target.id == 'counts' for target in node.targets))
        namespace = {'collections': types.SimpleNamespace(Counter=Counter), 'peer_id': 'B'}
        exec(compile(ast.Module(body=[assignment], type_ignores=[]), 'initial-counts', 'exec'), namespace)
        self.assertEqual(dict(namespace['counts']), {'self': 0, 'peer:B': 0, 'invalid': 0})

    def test_invalid_peer_never_changes_valid_counts_or_last_frame(self):
        tree = ast.parse((BASE / 'collector.py').read_text())
        functions = [node for node in tree.body if isinstance(node, ast.FunctionDef)]
        rows = []
        namespace = {'json': json, 'math': math, 'time': time, 'counts': Counter(), 'last': {},
                     'peer_id': 'B', 'last_peer_mono': None, 'max_gap': 0.0,
                     'emit': lambda kind, **fields: rows.append({'kind': kind, **fields})}
        exec(compile(ast.Module(body=[node for node in functions if node.name != 'emit'], type_ignores=[]), 'collector-functions', 'exec'), namespace)
        geometry = {'map_id': 'map-a', 'sampled_at_ns': 123, 'pose': [1, 2, 0], 'velocity': [0, 0, 0],
                    'footprint': [[0, 0], [1, 0], [0, 1]]}
        valid = {'robot_id': 'B', 'geometry': geometry}
        namespace['receive']('peer', types.SimpleNamespace(data=json.dumps(valid)))
        original_time = namespace['last_peer_mono']
        for bad in [{key: value for key, value in geometry.items() if key != 'sampled_at_ns'},
                    dict(geometry, sampled_at_ns='123'), dict(geometry, pose=[1, 2]),
                    dict(geometry, footprint=[[0, 0], [1, 0], [0, float('nan')]])]:
            raw = json.dumps({'robot_id': 'B', 'geometry': bad})
            namespace['receive']('peer', types.SimpleNamespace(data=raw))
            self.assertEqual(rows[-1]['kind'], 'invalid')
            self.assertEqual(rows[-1]['raw'], raw)
        self.assertEqual(namespace['counts'], {'peer:B': 1, 'invalid': 4})
        self.assertEqual(namespace['last']['peer'], valid)
        self.assertEqual(namespace['last_peer_mono'], original_time)

    def test_standby_commands_cannot_call_ssh(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tripwire = root / 'ssh-called'
            script = root / 'ssh'
            script.write_text('#!/bin/sh\ntouch ' + shlex.quote(str(tripwire)) + '\nexit 99\n')
            script.chmod(0o755)
            import os
            env = dict(os.environ, PATH=str(root) + ':' + os.environ['PATH'])
            (root / 'active.json').write_text(json.dumps({'monitor_mode': 'waiting_for_vehicles'}))
            for filename, args in [('start_recording.py', []), ('feedback.py', []), ('stop_recording.py', ['--stop'])]:
                (root / filename).write_text((BASE / filename).read_text())
                result = subprocess.run([sys.executable, str(root / filename)] + args, env=env, capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(tripwire.exists())

    def test_missing_and_duplicate_targets_fail_before_network(self):
        with mock.patch.object(START.subprocess, 'run') as run:
            with self.assertRaises(ValueError):
                START.validate_targets({'wx1': {'host': None}, 'wx2': {'host': None}})
            for field in ['host', 'robot_id', 'bluetooth_mac']:
                config = copy.deepcopy(VALID)
                config['wx2'][field] = config['wx1'][field]
                with self.assertRaises(ValueError):
                    START.validate_targets(config)
            run.assert_not_called()

    def test_connections_use_frozen_ip_and_socket_not_alias(self):
        with mock.patch.object(START.subprocess, 'run', return_value=types.SimpleNamespace(stdout='controlpath /tmp/verified-target\n')):
            args = START.freeze_connection(VALID['wx1'])
        self.assertEqual(args[-1], '192.0.2.10')
        self.assertIn('HostName=192.0.2.10', args)
        self.assertIn('/tmp/verified-target', args)
        self.assertNotIn('wx1', args)

    def test_waiting_view_has_no_fabricated_live_values(self):
        rows = VIEW.lines_for({'mode': 'waiting_for_vehicles'}, time.time_ns())
        text = '\n'.join(row[0] for row in rows)
        self.assertIn('等待车辆 · 未连接', text)
        self.assertNotIn('正在记录', text)
        self.assertFalse(any(row[0].endswith(' m') for row in rows))

    def test_stale_distance_and_invalid_pose_are_hidden(self):
        record = {'epoch_ns': 1, 'summary': {'distance_valid': True, 'distance_m': 2.0, 'vehicles': {
            'wx1': {'pose': {'x': 1000, 'y': 2000, 'theta': 0}, 'pose_valid': False}}}}
        rows = VIEW.lines_for(record, 20000000001)
        self.assertIn(('-- m', 'big'), rows)
        self.assertFalse(any(text.startswith('x ') for text, _ in rows))

    def test_embedded_reader_compiles_and_rejects_wrong_identity(self):
        tree = ast.parse((BASE / 'feedback.py').read_text())
        function = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == 'read')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = {'own_id': 'A', 'peer_id': 'B', 'mac': VALID['wx1']['bluetooth_mac'], 'ssh_base': ['ssh', '192.0.2.10']}
            raw = '{"kind":"self","value":"中文"}\n'.encode()
            wrong = False
            def fake_run(args, **kwargs):
                compile(shlex.split(args[-1])[-1], 'embedded-reader', 'exec')
                identity = {'own_id': 'WRONG' if wrong else 'A', 'expected_peer': 'B', 'own_mac': expected['mac'],
                            'finished': True, 'recorder_alive': False, 'remote_status_age_s': 0.1}
                header = {'status': identity, 'offset': 0, 'bytes': len(raw), 'remote_bytes': len(raw)}
                return types.SimpleNamespace(returncode=0, stdout=json.dumps(header).encode() + bytes([10]) + raw, stderr=b'')
            namespace = {'ROOT': root, 'REMOTE': '/tmp/test-recording', 'SESSION': {'targets': {'wx1': expected}},
                         'time': time, 'json': json, 'shlex': shlex, 'subprocess': types.SimpleNamespace(run=fake_run)}
            exec(compile(ast.Module(body=[function], type_ignores=[]), 'reader-function', 'exec'), namespace)
            namespace['read']('wx1')
            self.assertEqual((root / 'wx1-facts.jsonl').read_bytes(), raw)
            (root / 'wx1-facts.jsonl').unlink()
            wrong = True
            with self.assertRaises(RuntimeError):
                namespace['read']('wx1')
            self.assertFalse((root / 'wx1-facts.jsonl').exists())

    def test_finished_recorders_drain_all_raw_bytes_before_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            targets = {label: {'own_id': label, 'peer_id': 'wx2' if label == 'wx1' else 'wx1', 'mac': VALID[label]['bluetooth_mac'], 'ssh_base': ['ssh', label + '-fixed-ip']} for label in VALID}
            state = {'monitor_mode': 'recording', 'local_directory': directory, 'remote_directory': '/tmp/test', 'targets': targets}
            (base / 'active.json').write_text(json.dumps(state))
            cycles = 0
            def fake_run(args, **kwargs):
                label = args[-2].removesuffix('-fixed-ip')
                path = base / (label + '-facts.jsonl')
                offset = path.stat().st_size if path.exists() else 0
                chunk = b'ab'[offset:offset+1]
                identity = {'own_id': label, 'expected_peer': targets[label]['peer_id'], 'own_mac': targets[label]['mac'],
                            'finished': True, 'recorder_alive': False, 'remote_status_age_s': 0.1, 'counts': {'self': 1}, 'last': {}, 'epoch_ns': time.time_ns()}
                header = {'status': identity, 'offset': offset, 'bytes': len(chunk), 'remote_bytes': 2}
                return types.SimpleNamespace(returncode=0, stdout=json.dumps(header).encode() + bytes([10]) + chunk, stderr=b'')
            def advance(_):
                nonlocal cycles
                cycles += 1
                if cycles > 3:
                    raise RuntimeError('reader failed to finish draining')
            with mock.patch('subprocess.run', side_effect=fake_run), mock.patch('time.sleep', side_effect=advance), mock.patch('builtins.print'), mock.patch('sys.argv', ['feedback.py', '--run']):
                exec(compile((BASE / 'feedback.py').read_text(), str(BASE / 'feedback.py'), 'exec'), {'__file__': str(base / 'feedback.py'), '__name__': '__main__'})
            for label in VALID:
                self.assertEqual((base / (label + '-facts.jsonl')).read_bytes(), b'ab')
            self.assertGreaterEqual(cycles, 1)


    def test_feedback_without_run_stays_offline_even_if_session_is_recording(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'feedback.py').write_text((BASE / 'feedback.py').read_text())
            (root / 'active.json').write_text(json.dumps({'monitor_mode': 'recording'}))
            result = subprocess.run([sys.executable, str(root / 'feedback.py')], capture_output=True, text=True, timeout=3)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('待机', result.stdout)

    def test_feedback_rejects_bad_last_peer_without_losing_following_windows(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            targets = {label: {'own_id': label, 'peer_id': 'B', 'mac': VALID[label]['bluetooth_mac'], 'ssh_base': ['ssh', label]}
                       for label in VALID}
            (root / 'active.json').write_text(json.dumps({'monitor_mode': 'recording', 'local_directory': directory,
                'remote_directory': '/tmp/test', 'targets': targets}))
            cycle = 0
            samples = [1, 2, None, 3, 1, 'read_error', 5, 6]
            def read(args, **kwargs):
                label = args[-2]
                if samples[cycle] == 'read_error':
                    return types.SimpleNamespace(returncode=1, stdout=b'', stderr=b'offline read error')
                now = time.time_ns()
                counts = {'self': 1}
                if samples[cycle] is not None:
                    counts['peer:B'] = samples[cycle]
                value = {'own_id': label, 'expected_peer': 'B', 'own_mac': targets[label]['mac'], 'counts': counts,
                         'last': {'peer': {'geometry': {}}}, 'epoch_ns': now, 'recorder_alive': True,
                         'remote_status_age_s': 0, 'finished': False,
                         'vehicle': {'epoch_ns': now, 'status': {'localization': {'localized': True, 'pose': {
                             'x': 0, 'y': 0, 'theta': 0, 'updated_at': '2026-01-01T00:00:00Z'}}}}}
                header = {'status': value, 'offset': 0, 'bytes': 0, 'remote_bytes': 0}
                return types.SimpleNamespace(returncode=0, stdout=json.dumps(header).encode() + b'\n', stderr=b'')
            def advance(_):
                nonlocal cycle
                cycle += 1
                if cycle == len(samples):
                    (root / 'STOP').touch()
            with mock.patch('subprocess.run', side_effect=read), mock.patch('time.sleep', side_effect=advance), mock.patch('builtins.print'), mock.patch('sys.argv', ['feedback.py', '--run']):
                exec(compile((BASE / 'feedback.py').read_text(), str(BASE / 'feedback.py'), 'exec'), {'__file__': str(root / 'feedback.py'), '__name__': '__main__'})
            rows = [json.loads(line) for line in (root / 'feedback.jsonl').read_text().splitlines()]
            values = [row['summary']['vehicles']['wx1'] for row in rows]
            self.assertEqual([value.get('peer_frames_delta') for value in values], [None, 1, None, None, None, None, None, 1])
            self.assertTrue(all(value.get('last_peer_source_time_ns') is None and value.get('peer_fact_error') for value in values if 'error' not in value))
            self.assertTrue(all(not row['summary']['distance_valid'] for row in rows))

    def test_start_failure_preserves_unconfirmed_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'active.json').write_text(json.dumps({'monitor_mode': 'waiting_for_vehicles'}))
            (root / 'targets.json').write_text(json.dumps(VALID))
            for filename in ['collector.py', 'radio_collector.py', 'resource_probe.py']:
                (root / filename).write_text('pass')
            def deploy(target, code, payload, container=False):
                state = json.loads((root / 'active.json').read_text())
                self.assertTrue((Path(state['local_directory']) / 'session.json').is_file())
                if target['own_id'] == 'B':
                    raise RuntimeError('simulated deployment failure')
                return {'pid': 123, 'directory': '/tmp/offline-test'}
            with mock.patch.object(START, 'BASE', root), mock.patch.object(START, 'freeze_connection', return_value=['ssh', 'fixed-ip']), mock.patch.object(START, 'remote_json', side_effect=deploy), mock.patch.object(STOP, 'request_stop', side_effect=RuntimeError('stop not confirmed')), mock.patch.dict(sys.modules, {'stop_recording': STOP}), mock.patch('sys.argv', ['start_recording.py', '--start']):
                with self.assertRaises(RuntimeError):
                    START.main()
            state = json.loads((root / 'active.json').read_text())
            self.assertEqual(state['monitor_mode'], 'cleanup_pending')
            self.assertFalse(any(item['confirmed'] for item in state['cleanup_stop_requests'].values()))
            saved = json.loads((Path(state['local_directory']) / 'session.json').read_text())
            self.assertEqual(saved['monitor_mode'], 'cleanup_pending')
            self.assertEqual(saved['targets'], state['targets'])

    def test_stop_halts_local_polling_even_when_remote_stop_is_unconfirmed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = {'monitor_mode': 'recording', 'local_directory': directory, 'remote_directory': '/tmp/offline-test', 'targets': {}}
            (root / 'active.json').write_text(json.dumps(state))
            with mock.patch.object(STOP, 'BASE', root), mock.patch.object(STOP, 'request_stop', side_effect=RuntimeError('unreachable')), mock.patch('sys.argv', ['stop_recording.py', '--stop']):
                with self.assertRaises(RuntimeError):
                    STOP.main()
            self.assertTrue((root / 'STOP').exists())
            self.assertEqual(json.loads((root / 'active.json').read_text())['monitor_mode'], 'cleanup_pending')

    def test_stop_rejects_wrong_source_before_completion(self):
        session = {'remote_directory': '/tmp/offline-test', 'targets': {'wx1': {'own_id': 'A', 'peer_id': 'B', 'mac': '02:00:00:00:00:01'}}}
        wrong = {'own_id': 'B', 'expected_peer': 'A', 'own_mac': '02:00:00:00:00:02', 'finished': True, 'alive': False}
        with mock.patch.object(STOP, 'ssh', return_value=types.SimpleNamespace(stdout=json.dumps(wrong).encode())) as run:
            with self.assertRaises(RuntimeError):
                STOP.finished(session, 'wx1')
            self.assertEqual(run.call_count, 1)

    def test_unstarted_components_can_confirm_absence_without_inventing_records(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            mac = VALID['wx1']['bluetooth_mac']
            session = {'remote_directory': str(root / 'never-started'), 'targets': {'wx1': {'own_id': 'A', 'peer_id': 'B', 'mac': mac}}}
            def execute(_session, _alias, command, **kwargs):
                code = shlex.split(command)[-1]
                capture = io.StringIO()
                response = io.BytesIO(json.dumps({'data': {'bluetooth': {'address': mac}}}).encode())
                with mock.patch('urllib.request.urlopen', return_value=response), mock.patch('sys.stdout', capture):
                    exec(compile(code, 'absence-inspection', 'exec'), {})
                return types.SimpleNamespace(stdout=capture.getvalue().encode())
            with mock.patch.object(STOP, 'ssh', side_effect=execute):
                self.assertTrue(STOP.finished(session, 'wx1'))
                self.assertTrue(STOP.inspect_source(session, 'wx1')['absent'])
                self.assertTrue(STOP.inspect_radio(session, 'wx1')['absent'])

    def test_archive_identity_check_precedes_archive_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.mkdir()
            (source / 'launch.json').write_text(json.dumps({'own_id': 'WRONG', 'peer_id': 'A', 'mac': '02:00:00:00:00:02'}))
            session = {'remote_directory': str(source), 'local_directory': directory, 'targets': {'wx1': {'own_id': 'A', 'peer_id': 'B', 'mac': '02:00:00:00:00:01', 'ssh_base': ['ssh', 'fixed-ip']}}}
            def run(args, **kwargs):
                code = shlex.split(args[-1])[-1]
                captured = io.BytesIO()
                with mock.patch('sys.stdout', types.SimpleNamespace(buffer=captured)):
                    try:
                        exec(compile(code, 'offline-archive', 'exec'), {})
                    except RuntimeError:
                        self.assertEqual(captured.getvalue(), b'')
                        return types.SimpleNamespace(returncode=1)
                raise AssertionError('wrong identity accepted')
            with mock.patch.object(STOP.subprocess, 'run', side_effect=run), mock.patch.object(STOP, 'inspect_source', return_value={'finished': True, 'alive': False}):
                with self.assertRaises(RuntimeError):
                    STOP.download(session, 'wx1')
            self.assertFalse((root / 'wx1-source.tar.gz').exists())

    def test_unstarted_components_do_not_create_fake_archives(self):
        with tempfile.TemporaryDirectory() as directory:
            session = {'local_directory': directory, 'remote_directory': '/tmp/not-started', 'targets': {'wx1': {'own_id': 'A', 'peer_id': 'B', 'mac': VALID['wx1']['bluetooth_mac']}}}
            with mock.patch.object(STOP, 'inspect_source', return_value={'absent': True}), mock.patch.object(STOP, 'inspect_radio', return_value={'absent': True}), mock.patch.object(STOP, 'ssh') as run:
                alias, result = STOP.download(session, 'wx1')
                self.assertEqual(alias, 'wx1')
                self.assertTrue(result['source']['not_started'])
                self.assertTrue(result['radio']['not_started'])
                run.assert_not_called()
                self.assertEqual(list(Path(directory).iterdir()), [])

    def test_successful_archives_are_hashed_with_supported_python(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            radio = root / 'source-radio'
            identity = {'own_id': 'A', 'peer_id': 'B', 'mac': VALID['wx1']['bluetooth_mac'], 'pid': 123}
            for folder in [source, radio]:
                folder.mkdir()
                (folder / 'launch.json').write_text(json.dumps(identity))
            session = {'remote_directory': str(source), 'local_directory': directory, 'targets': {'wx1': {**identity, 'ssh_base': ['ssh', 'fixed-ip']}}}
            def run(args, **kwargs):
                output = io.BytesIO()
                with mock.patch('sys.stdout', types.SimpleNamespace(buffer=output)):
                    exec(compile(shlex.split(args[-1])[-1], 'offline-export', 'exec'), {})
                kwargs['stdout'].write(output.getvalue())
                return types.SimpleNamespace(returncode=0)
            with mock.patch.object(STOP, 'inspect_source', return_value={'finished': True, 'alive': False}), mock.patch.object(STOP, 'inspect_radio', return_value={'finished': True, 'alive': False}), mock.patch.object(STOP.subprocess, 'run', side_effect=run):
                _, archives = STOP.download(session, 'wx1')
            import hashlib
            for archive in archives.values():
                data = Path(archive['path']).read_bytes()
                self.assertEqual(archive['sha256'], hashlib.sha256(data).hexdigest())
                self.assertEqual(archive['bytes'], len(data))

    def test_radio_partial_start_still_stops_btmon(self):
        tree = ast.parse((BASE / 'radio_collector.py').read_text())
        block = next(node for node in tree.body if isinstance(node, ast.Try))
        class Child:
            signals = []
            def poll(self): return None
            def send_signal(self, sig): self.signals.append(sig)
            def wait(self, timeout): return 0
        child = Child()
        calls = 0
        def popen(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 1: return child
            raise OSError('simulated resource startup failure')
        with tempfile.TemporaryDirectory() as directory:
            namespace = {'root': Path(directory), 'children': [], 'files': [], 'profile': None, 'duration': 1,
                         'subprocess': types.SimpleNamespace(Popen=popen, STDOUT=-2, TimeoutExpired=subprocess.TimeoutExpired), 'signal': signal, 'json': json, 'time': time}
            with self.assertRaises(OSError):
                exec(compile(ast.Module(body=[block], type_ignores=[]), 'radio-cleanup', 'exec'), namespace)
        self.assertEqual(child.signals, [signal.SIGINT])

if __name__ == '__main__':
    unittest.main()
