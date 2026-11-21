"""每五秒只读回传摘要；原始高频数据保存在车端并在结束后取回。"""
import concurrent.futures
import datetime
import json
import math
from pathlib import Path
import shlex
import subprocess
import sys
import time

BASE = Path(__file__).resolve().parent
if sys.argv[1:] != ['--run']:
    print('监控待机：明确开始后才采样。')
    raise SystemExit(0)
SESSION = json.loads((BASE / 'active.json').read_text())
if SESSION.get('monitor_mode') != 'recording':
    print('监控待机：车辆准备并明确开始后才采样。')
    raise SystemExit(0)
ROOT = Path(SESSION['local_directory'])
REMOTE = SESSION['remote_directory']
previous = {}
executor = concurrent.futures.ThreadPoolExecutor(max_workers=2)

def read(alias):
    archive = ROOT / (alias + '-facts.jsonl')
    offset = archive.stat().st_size if archive.exists() else 0
    remote = f"""import json,sys,time
from pathlib import Path
root=Path({REMOTE!r});data=json.loads((root/'status.json').read_text());raw=root/'facts.jsonl';size=raw.stat().st_size
if size<{offset}:raise RuntimeError('recording file shrank')
with raw.open('rb') as source:source.seek({offset});chunk=source.read(min(4194304 if data.get('finished') else 262144,size-{offset}))
pid=Path('/proc')/str(data['pid']);data['recorder_alive']=pid.exists() and str(root/'collector.py').encode() in (pid/'cmdline').read_bytes().split(bytes([0]))
data['remote_status_age_s']=(time.time_ns()-data['epoch_ns'])/1e9
header={{'status':data,'offset':{offset},'bytes':len(chunk),'remote_bytes':size}}
sys.stdout.buffer.write(json.dumps(header,ensure_ascii=True).encode()+bytes([10])+chunk)
"""
    command = 'docker exec easygo python3 -c ' + shlex.quote(remote)
    args = SESSION['targets'][alias]['ssh_base'] + [command]
    start = time.time_ns()
    result = subprocess.run(args, capture_output=True, timeout=4)
    end = time.time_ns()
    if result.returncode:
        return alias, {'error': result.stderr.decode(errors='replace'), 'query_start_ns': start, 'query_end_ns': end}
    header, chunk = result.stdout.split(b'\n', 1)
    header = json.loads(header)
    identity = header['status']
    expected = SESSION['targets'][alias]
    if (identity.get('own_id'), identity.get('expected_peer'), identity.get('own_mac')) != (expected['own_id'], expected['peer_id'], expected['mac']):
        raise RuntimeError('recording identity mismatch')
    if header['offset'] != offset or header['bytes'] != len(chunk):
        raise RuntimeError('recording byte offset/count mismatch')
    with archive.open('ab') as destination:
        destination.write(chunk)
    data = header['status']
    data['archived_raw_bytes'] = offset + len(chunk)
    data['remote_raw_bytes'] = header['remote_bytes']
    if not data.get('finished') and (not data['recorder_alive'] or not -0.2 <= data['remote_status_age_s'] <= 2):
        data['error'] = 'recorder heartbeat stale or process exited'
    data['query_start_ns'], data['query_end_ns'] = start, end
    return alias, data

start = time.monotonic()
index = 0
with (ROOT / 'feedback.jsonl').open('a') as full, (ROOT / 'feedback.stderr').open('a') as errors:
    while not (ROOT / 'STOP').exists():
        now = time.time_ns()
        data = {}
        for requested_alias, future in [(alias, executor.submit(read, alias)) for alias in ['wx1', 'wx2']]:
            try:
                alias, value = future.result()
                data[alias] = value
            except Exception as error:
                data[requested_alias] = {'error': str(error)}
                errors.write(requested_alias + ': ' + str(error) + '\n')
                errors.flush()
        row = {'epoch_ns': now, 'elapsed_s': time.monotonic() - start, 'vehicles': data}
        compact = {'local_time': datetime.datetime.now().strftime('%H:%M:%S'), 'distance_m': None, 'distance_valid': False, 'distance_estimated': False, 'vehicles': {}}
        positions = {}
        for alias in ['wx1', 'wx2']:
            value = data.get(alias, {})
            if 'error' in value or 'counts' not in value:
                previous.pop(alias, None)
                compact['vehicles'][alias] = {'error': value.get('error', 'read failed')}
                continue
            peer = SESSION['targets'][alias]['peer_id']
            received = value['counts'].get('peer:' + peer)
            if type(received) is not int or received < 0:
                received = None
            prior = previous.get(alias)
            delta = received - prior if received is not None and prior is not None and received >= prior else None
            previous[alias] = received
            api = value.get('vehicle')
            status = api.get('status') if isinstance(api, dict) else None
            loc = status.get('localization') if isinstance(status, dict) else None
            loc = loc if isinstance(loc, dict) else {}
            pose = loc.get('pose')
            pose = pose if isinstance(pose, dict) else None
            item = {'own_id': value.get('own_id'), 'self_frames': value['counts'].get('self', 0), 'peer_frames_total': received, 'peer_frames_delta': delta,
                    'no_peer_for_s': value.get('last_peer_gap_s'), 'pose': pose, 'localized': loc.get('localized'),
                    'api_error': value.get('api_error'), 'finished': value.get('finished'), 'archived_raw_bytes': value.get('archived_raw_bytes'), 'remote_raw_bytes': value.get('remote_raw_bytes')}
            fact = value.get('last', {}).get('peer')
            geometry = fact.get('geometry') if isinstance(fact, dict) else None
            sampled = geometry.get('sampled_at_ns') if isinstance(geometry, dict) else None
            item['last_peer_source_time_ns'] = sampled if type(sampled) is int and 0 <= sampled < 2 ** 63 else None
            if fact is not None and item['last_peer_source_time_ns'] is None:
                item['peer_fact_error'] = 'last peer has invalid sample timestamp/geometry'
            compact['vehicles'][alias] = item
            item['pose_valid'] = False
            item['pose_age_s'] = None
            confidence = pose.get('confidence') if pose else None
            if api and pose and loc.get('localized') and not value.get('api_error') and isinstance(pose.get('map_id'), str) and pose['map_id'] and all(type(pose.get(key)) in (int, float) and math.isfinite(pose[key]) for key in ['x', 'y', 'theta']):
                try:
                    api_age = (value['epoch_ns'] - api['epoch_ns']) / 1e9
                    pose_epoch = datetime.datetime.fromisoformat(pose['updated_at'].replace('Z', '+00:00')).timestamp()
                    pose_age = value['epoch_ns'] / 1e9 - pose_epoch
                    item['pose_age_s'] = pose_age
                    if 0 <= api_age <= 2 and -0.2 <= pose_age <= 2:
                        positions[alias] = pose
                        item['pose_valid'] = True
                        item['low_confidence'] = type(confidence) not in (int, float) or not math.isfinite(confidence) or confidence < 40
                except (KeyError, TypeError, ValueError):
                    pass

        if len(positions) == 2 and positions['wx1']['map_id'] == positions['wx2']['map_id']:
            a, b = positions['wx1'], positions['wx2']
            compact['distance_m'] = math.hypot(a['x'] - b['x'], a['y'] - b['y']) / 1000
            compact['distance_valid'] = True
            compact['distance_estimated'] = any(v.get('low_confidence') for v in compact['vehicles'].values())
        row['summary'] = compact
        full.write(json.dumps(row, ensure_ascii=False) + '\n')
        full.flush()
        tmp = ROOT / 'latest.tmp'
        tmp.write_text(json.dumps(row, ensure_ascii=False, indent=2))
        tmp.replace(ROOT / 'latest.json')
        print(json.dumps(compact, ensure_ascii=False), flush=True)
        if all(data.get(alias, {}).get('finished') and data[alias].get('archived_raw_bytes') == data[alias].get('remote_raw_bytes') for alias in ['wx1', 'wx2']):
            break
        index += 1
        time.sleep(max(0, start + index * 5 - time.monotonic()))
executor.shutdown(wait=True)
