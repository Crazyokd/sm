"""显式 --start 后才连接车辆；地址与身份固定在本次会话中。"""
import concurrent.futures
import datetime
import fcntl
import ipaddress
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time
import uuid

BASE = Path(__file__).resolve().parent
LABELS = ('wx1', 'wx2')


def validate_targets(config):
    targets = {}
    for label in LABELS:
        item = dict(config.get(label, {}))
        host = item.get('host')
        if not host:
            raise ValueError(f'{label}: 等待车辆 IP 地址')
        item['host'] = str(ipaddress.IPv4Address(host))
        if not isinstance(item.get('robot_id'), str) or not item['robot_id'].strip():
            raise ValueError(f'{label}: 等待车辆 robot_id')
        mac = item.get('bluetooth_mac')
        if not isinstance(mac, str) or not re.fullmatch(r'(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}', mac):
            raise ValueError(f'{label}: 等待有效蓝牙 MAC')
        item['bluetooth_mac'] = mac.upper()
        if not isinstance(item.get('ssh_user'), str) or not item['ssh_user']:
            raise ValueError(f'{label}: SSH 用户不能为空')
        if not isinstance(item.get('ssh_port'), int) or not 1 <= item['ssh_port'] <= 65535:
            raise ValueError(f'{label}: SSH 端口无效')
        targets[label] = item
    for field in ['host', 'robot_id', 'bluetooth_mac']:
        if targets['wx1'][field] == targets['wx2'][field]:
            raise ValueError(f'两车 {field} 重复')
    return targets


def freeze_connection(item):
    host = item['host']
    args = ['ssh', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=4', '-o', 'HostName=' + host,
            '-l', item['ssh_user'], '-p', str(item['ssh_port'])]
    config = subprocess.run(args + ['-G', host], capture_output=True, text=True, check=True).stdout
    values = dict(line.split(' ', 1) for line in config.splitlines() if ' ' in line)
    socket = values.get('controlpath')
    if socket and socket != 'none':
        args += ['-S', socket]
    return args + [host]


INSTALL = r'''import json, os, re, subprocess, sys, urllib.request
from pathlib import Path
p = json.load(sys.stdin)
target = p['target']
root = Path(p['directory'])
candidates = []
with urllib.request.urlopen('http://127.0.0.1:18088/api/v6/vehicle/status', timeout=3) as response:
    s = json.load(response)['data']
if not s['bluetooth']['address'] == target['mac']:
    raise RuntimeError('vehicle MAC mismatch')
for folder in Path('/proc').iterdir():
    if not folder.name.isdigit():
        continue
    try:
        args = (folder / 'cmdline').read_bytes().split(bytes([0]))
        if args[0] == b'/opt/sros/bin/peer-information' and args[1].decode() == target['own_id']:
            candidates.append(folder)
    except (OSError, IndexError, UnicodeError):
        pass
if not len(candidates) == 1:
    raise RuntimeError(('peer-information identity/count mismatch', len(candidates)))
env = dict((v.split('=', 1) for v in (candidates[0] / 'environ').read_bytes().decode().split(chr(0)) if '=' in v))
if 'CYCLONEDDS_URI' in env:
    env['CYCLONEDDS_URI'] = re.sub('<MaxAutoParticipantIndex>.*?</MaxAutoParticipantIndex>', '<MaxAutoParticipantIndex>300</MaxAutoParticipantIndex>', env['CYCLONEDDS_URI'])
root.mkdir(parents=True, exist_ok=True)
(root / 'collector.py').write_text(p['source'])
if (root / 'STOP').exists():
    raise RuntimeError('startup already cancelled')
with (root / 'facts.jsonl').open('w') as out, (root / 'stderr.log').open('w') as err:
    child = subprocess.Popen(['/usr/bin/python3', '-u', str(root / 'collector.py'), target['peer_id'], target['own_id'], target['mac'], str(root)], env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=err, start_new_session=True)
versions = subprocess.run(['dpkg-query', '-W', 'easygo-app', 'navigation2', 'easygo-param-interfaces'], capture_output=True, text=True).stdout
result = {'pid': child.pid, 'directory': str(root), 'own_id': target['own_id'], 'peer_id': target['peer_id'], 'mac': target['mac'], 'package_versions': versions, 'transport': 'udp' if env.get('EASYGO_BLUETOOTH_TRANSPORT') == 'udp' else 'bluez', 'transport_env': env.get('EASYGO_BLUETOOTH_TRANSPORT')}
(root / 'launch.json').write_text(json.dumps(result))
print(json.dumps(result))
'''

RADIO_INSTALL = r'''import json, subprocess, sys, urllib.request
from pathlib import Path
p = json.load(sys.stdin)
target = p['target']
root = Path(p['directory'])
with urllib.request.urlopen('http://127.0.0.1:18088/api/v6/vehicle/status', timeout=3) as response:
    status = json.load(response)['data']
if not status['bluetooth']['address'] == target['mac']:
    raise RuntimeError('radio vehicle MAC mismatch')
root.mkdir(parents=True, exist_ok=True)
(root / 'radio_collector.py').write_text(p['radio'])
(root / 'resource_probe.py').write_text(p['resource'])
if (root / 'STOP').exists():
    raise RuntimeError('radio startup already cancelled')
with (root / 'launcher.stdout').open('w') as out, (root / 'launcher.stderr').open('w') as err:
    child = subprocess.Popen(['sudo', '-n', '/usr/bin/python3', '-u', str(root / 'radio_collector.py'), str(root), p['peer_mac'], '7200'], stdin=subprocess.DEVNULL, stdout=out, stderr=err, start_new_session=True)
result = {'pid': child.pid, 'directory': str(root), 'own_id': target['own_id'], 'peer_id': target['peer_id'], 'mac': target['mac']}
(root / 'launch.json').write_text(json.dumps(result))
print(json.dumps(result))
'''



READY = r'''import json, subprocess, sys, time
from pathlib import Path
p = json.load(sys.stdin)
deadline = time.monotonic() + 12
last = 'not ready'
while time.monotonic() < deadline:
    try:
        result = subprocess.run(['docker', 'exec', 'easygo', 'cat', p['directory'] + '/status.json'], capture_output=True, timeout=2, check=True)
        source = json.loads(result.stdout)
        radio = json.loads((Path(p['directory'] + '-radio') / 'status.json').read_text())
        if not (source['own_id'] == p['own_id'] and source['own_mac'] == p['mac']):
            raise RuntimeError('recorder identity mismatch')
        if not source['expected_peer'] == p['peer_id']:
            raise RuntimeError('peer identity mismatch')
        if not source['finished'] and time.time_ns() - source['epoch_ns'] < 3000000000 and radio['hci_process_alive'] and radio['resource_process_alive']:
            print(json.dumps({'ready': True}))
            break
    except (OSError, ValueError, KeyError, subprocess.SubprocessError, AssertionError) as error:
        last = str(error)
    time.sleep(0.25)
else:
    raise RuntimeError('recorders not ready: ' + last)
'''


def remote_json(target, code, payload, container=False):
    prefix = 'docker exec -i easygo ' if container else ''
    command = prefix + 'python3 -c ' + shlex.quote(code)
    result = subprocess.run(target['ssh_base'] + [command], input=json.dumps(payload), text=True,
                            capture_output=True, timeout=20, check=True)
    return json.loads(result.stdout)


def main():
    if sys.argv[1:] != ['--start']:
        print('待机：配置 targets.json 后，明确使用 --start 才会连接车辆。')
        return
    with (BASE / 'start.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        active_file = BASE / 'active.json'
        old = json.loads(active_file.read_text())
        if old.get('monitor_mode') in ['recording', 'starting', 'cleanup_pending', 'stopping']:
            raise RuntimeError('已有记录会话，请先停止该会话')
        config = validate_targets(json.loads((BASE / 'targets.json').read_text()))
        targets = {}
        for label in LABELS:
            peer = 'wx2' if label == 'wx1' else 'wx1'
            targets[label] = {'own_id': config[label]['robot_id'], 'peer_id': config[peer]['robot_id'],
                              'mac': config[label]['bluetooth_mac'], 'host': config[label]['host'],
                              'ssh_base': freeze_connection(config[label])}
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
        session_id = stamp + '-' + uuid.uuid4().hex[:8]
        local = BASE / 'recordings' / session_id
        local.mkdir(parents=True)
        remote = '/tmp/easygo-ble-move-' + session_id
        state = {'monitor_mode': 'starting', 'started_utc': stamp, 'local_directory': str(local),
                 'remote_directory': remote, 'targets': targets, 'max_duration_seconds': 7200}
        (local / 'latest.json').write_text(json.dumps({'mode': 'starting'}))
        active_file.write_text(json.dumps(state, ensure_ascii=False, indent=2))
        (local / 'session.json').write_text(json.dumps(state, ensure_ascii=False, indent=2))
        try:
            def deploy(label):
                target = targets[label]
                source = remote_json(target, INSTALL, {'target': target, 'directory': remote,
                                                      'source': (BASE / 'collector.py').read_text()}, container=True)
                target.update(source)
                peer = 'wx2' if label == 'wx1' else 'wx1'
                radio = remote_json(target, RADIO_INSTALL, {'directory': remote + '-radio', 'target': target,
                                   'peer_mac': targets[peer]['mac'], 'radio': (BASE / 'radio_collector.py').read_text(),
                                   'resource': (BASE / 'resource_probe.py').read_text()})
                remote_json(target, READY, {'directory': remote, 'own_id': target['own_id'], 'peer_id': target['peer_id'], 'mac': target['mac']})
                return label, radio
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                state['radio'] = dict(pool.map(deploy, LABELS))
            state['monitor_mode'] = 'recording'
            active_file.write_text(json.dumps(state, ensure_ascii=False, indent=2))
            (local / 'session.json').write_text(json.dumps(state, ensure_ascii=False, indent=2))
            with (local / 'reader.stdout').open('a') as out, (local / 'reader.stderr').open('a') as err:
                child = subprocess.Popen([sys.executable, '-u', str(BASE / 'feedback.py'), '--run'], stdin=subprocess.DEVNULL,
                                         stdout=out, stderr=err, start_new_session=True)
            state['reader_pid'] = child.pid
            active_file.write_text(json.dumps(state, ensure_ascii=False, indent=2))
            print(json.dumps({'started': True, 'directory': str(local), 'targets': targets}, ensure_ascii=False, indent=2))
        except BaseException as error:
            state['monitor_mode'] = 'cleanup_pending'
            state['startup_error'] = str(error)
            state['cleanup_stop_requests'] = {}
            active_file.write_text(json.dumps(state, ensure_ascii=False, indent=2))
            from stop_recording import request_stop
            for label in targets:
                try:
                    state['cleanup_stop_requests'][label] = request_stop(state, label)
                except Exception as cleanup_error:
                    state['cleanup_stop_requests'][label] = {'requested': False, 'confirmed': False, 'error': str(cleanup_error)}
                active_file.write_text(json.dumps(state, ensure_ascii=False, indent=2))
            (local / 'session.json').write_text(json.dumps(state, ensure_ascii=False, indent=2))
            raise


if __name__ == '__main__':
    main()
