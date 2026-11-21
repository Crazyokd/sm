"""显式停止本次只读采样，确认身份与结束状态后取回完整记录。"""
import concurrent.futures
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time

BASE = Path(__file__).resolve().parent


def verify_identity(target, record):
    if (record.get('own_id'), record.get('expected_peer'), record.get('own_mac')) != (target['own_id'], target['peer_id'], target['mac']):
        raise RuntimeError('recording identity mismatch')


def ssh(session, alias, command, **kwargs):
    result = subprocess.run(session['targets'][alias]['ssh_base'] + [command],
                            timeout=kwargs.pop('timeout', 20), **kwargs)
    if result.returncode:
        raise RuntimeError(f'{alias}: command exited {result.returncode}')
    return result



ABSENCE_CHECK = r'''import json, urllib.request
from pathlib import Path

def confirm_absent(root, scripts, data_files, expected_mac):
    for folder in Path('/proc').iterdir():
        if not folder.name.isdigit():
            continue
        try:
            argv = (folder / 'cmdline').read_bytes().split(bytes([0]))
            if any((str(root / name).encode() in argv for name in scripts)):
                raise RuntimeError('owned process exists without launch metadata')
        except (OSError, ValueError):
            pass
    for (name, header_bytes) in data_files:
        path = root / name
        if path.is_file() and path.stat().st_size > header_bytes:
            raise RuntimeError('unattributed recording data exists; keep cleanup pending')
    with urllib.request.urlopen('http://127.0.0.1:18088/api/v6/vehicle/status', timeout=3) as response:
        actual = json.load(response)['data']['bluetooth']['address']
    if not actual == expected_mac:
        raise RuntimeError('absence endpoint MAC mismatch')
    return {'absent': True, 'finished': True, 'alive': False, 'hardware_mac': actual}
'''


def inspect_source(session, alias):
    remote = session['remote_directory']
    code = ABSENCE_CHECK + f"""
root=Path({remote!r});launch_path=root/'launch.json';path=root/'status.json'
if not launch_path.exists() and not path.exists():
 result=confirm_absent(root,['collector.py'],[('facts.jsonl',0)],{session['targets'][alias]['mac']!r})
else:
 launch=json.loads(launch_path.read_text()) if launch_path.exists() else None
 s=json.loads(path.read_text()) if path.exists() else {{'own_id':launch['own_id'],'expected_peer':launch['peer_id'],'own_mac':launch['mac'],'finished':False,'pid':launch['pid']}}
 pid=Path('/proc')/str(s.get('pid',launch['pid'] if launch else None))
 try:alive=str(root/'collector.py').encode() in (pid/'cmdline').read_bytes().split(bytes([0]))
 except OSError:alive=False
 result={{'own_id':s['own_id'],'expected_peer':s['expected_peer'],'own_mac':s['own_mac'],'finished':s.get('finished',False),'alive':alive}}
print(json.dumps(result))
"""
    response = ssh(session, alias, 'docker exec easygo python3 -c ' + shlex.quote(code), capture_output=True)
    source = json.loads(response.stdout)
    if source.get('absent'):
        if source.get('hardware_mac') != session['targets'][alias]['mac']:
            raise RuntimeError('absence identity mismatch')
    else:
        verify_identity(session['targets'][alias], source)
    return source


def inspect_radio(session, alias):
    remote = session['remote_directory']
    radio = remote + '-radio'
    code = ABSENCE_CHECK + f"""
root=Path({radio!r});launch_path=root/'launch.json'
if not launch_path.exists():
 result=confirm_absent(root,['radio_collector.py','resource_probe.py','hci.btsnoop'],[('radio.jsonl',0),('resources.jsonl',0),('hci.btsnoop',16)],{session['targets'][alias]['mac']!r})
else:
 launch=json.loads(launch_path.read_text());pid=Path('/proc')/str(launch['pid'])
 try:alive=str(root/'radio_collector.py').encode() in (pid/'cmdline').read_bytes().split(bytes([0]))
 except OSError:alive=False
 try:done=json.loads((root/'finished.json').read_text()).get('finished',False)
 except (OSError,ValueError):done=False
 result={{'own_id':launch['own_id'],'expected_peer':launch['peer_id'],'own_mac':launch['mac'],'finished':done,'alive':alive}}
print(json.dumps(result))
"""
    response = ssh(session, alias, 'python3 -c ' + shlex.quote(code), capture_output=True)
    radio_status = json.loads(response.stdout)
    if radio_status.get('absent'):
        if radio_status.get('hardware_mac') != session['targets'][alias]['mac']:
            raise RuntimeError('radio absence identity mismatch')
    else:
        verify_identity(session['targets'][alias], radio_status)
    return radio_status


def inspect(session, alias):
    return inspect_source(session, alias), inspect_radio(session, alias)


def request_stop(session, alias):
    # 校验旧会话身份后才写停止标记，避免连接意外指向其他车时停止错误任务。
    inspect_source(session, alias)
    remote = session['remote_directory']
    radio = remote + '-radio'
    cancel = 'from pathlib import Path; p=Path(' + repr(remote) + '); p.mkdir(parents=True,exist_ok=True); (p/"STOP").touch()'
    ssh(session, alias, 'docker exec easygo python3 -c ' + shlex.quote(cancel), capture_output=True)
    expected = {key: session['targets'][alias][key] for key in ['own_id', 'peer_id', 'mac']}
    code = f"""from pathlib import Path
import os,signal,subprocess,json
root=Path({radio!r});resource=str(root/'resource_probe.py').encode()
identity=root/'launch.json'
if identity.exists():
 record=json.loads(identity.read_text());expected={expected!r}
 if not all(record.get(k)==v for k,v in expected.items()):raise RuntimeError('radio stop identity mismatch')
for p in Path('/proc').iterdir():
 if not p.name.isdigit():continue
 try:
  args=(p/'cmdline').read_bytes().split(bytes([0]))
  if resource in args:os.kill(int(p.name),signal.SIGTERM)
 except (OSError,ValueError):pass
subprocess.run(['docker','exec','easygo','touch',{(remote+'/STOP')!r}],check=True)
root.mkdir(parents=True,exist_ok=True);(root/'STOP').touch()
"""
    ssh(session, alias, 'sudo -n python3 -c ' + shlex.quote(code), capture_output=True)
    return {'requested': True, 'confirmed': False}


def finished(session, alias):
    source, radio = inspect(session, alias)
    return all(value['finished'] or not value['alive'] for value in [source, radio])


def download(session, alias):
    target = session['targets'][alias]
    results = {}
    for kind, directory, prefix in [('source', session['remote_directory'], 'docker exec easygo'),
                                     ('radio', session['remote_directory'] + '-radio', 'sudo -n')]:
        inspection = inspect_source(session, alias) if kind == 'source' else inspect_radio(session, alias)
        if inspection.get('absent'):
            results[kind] = {'not_started': True}
            continue
        # 身份检查与归档处于同一个远端命令，防止取回时把另一台车的数据贴错标签。
        expected = {'own_id': target['own_id'], 'peer_id': target['peer_id'], 'mac': target['mac']}
        code = f"""from pathlib import Path
import json,sys,tarfile
root=Path({directory!r});expected={expected!r}
if (root/'launch.json').exists():launch=json.loads((root/'launch.json').read_text())
else:
 s=json.loads((root/'status.json').read_text());launch={{'own_id':s['own_id'],'peer_id':s['expected_peer'],'mac':s['own_mac']}}
if not all(launch.get(k)==v for k,v in expected.items()):raise RuntimeError('archive identity mismatch')
with tarfile.open(fileobj=sys.stdout.buffer,mode='w|gz') as archive:archive.add(root,arcname='.')
"""
        destination = Path(session['local_directory']) / f'{alias}-{kind}.tar.gz'
        temporary = destination.with_suffix('.partial')
        with temporary.open('wb') as output:
            ssh(session, alias, prefix + ' python3 -c ' + shlex.quote(code), stdout=output, timeout=180)
        temporary.replace(destination)
        with destination.open('rb') as captured:
            hasher = hashlib.sha256()
            for chunk in iter(lambda: captured.read(1024 * 1024), b''):
                hasher.update(chunk)
            digest = hasher.hexdigest()
        results[kind] = {'path': str(destination), 'bytes': destination.stat().st_size, 'sha256': digest}
    return alias, results


def main():
    if sys.argv[1:] != ['--stop']:
        print('待机；记录开始后使用 --stop 停止并归档。')
        return
    active = BASE / 'active.json'
    session = json.loads(active.read_text())
    if session.get('monitor_mode') not in ['recording', 'starting', 'cleanup_pending', 'stopping']:
        print('当前待机，没有正在运行的新监控会话。')
        return
    (Path(session['local_directory']) / 'STOP').touch()
    session['monitor_mode'] = 'stopping'
    active.write_text(json.dumps(session, ensure_ascii=False, indent=2))
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            list(pool.map(lambda alias: request_stop(session, alias), ['wx1', 'wx2']))
            deadline = time.monotonic() + 70
            while time.monotonic() < deadline:
                states = dict(zip(['wx1', 'wx2'], pool.map(lambda alias: finished(session, alias), ['wx1', 'wx2'])))
                if all(states.values()):
                    break
                print(json.dumps({'stopping': states}), flush=True)
                time.sleep(1)
            else:
                raise RuntimeError('Recorders have not all confirmed stopping')
            archives = dict(pool.map(lambda alias: download(session, alias), ['wx1', 'wx2']))
    except BaseException as error:
        session['monitor_mode'] = 'cleanup_pending'
        session['cleanup_error'] = str(error)
        active.write_text(json.dumps(session, ensure_ascii=False, indent=2))
        raise
    result = {'stopped_epoch_ns': time.time_ns(), 'archives': archives}
    (Path(session['local_directory']) / 'archives.json').write_text(json.dumps(result, ensure_ascii=False, indent=2))
    session['monitor_mode'] = 'stopped'
    active.write_text(json.dumps(session, ensure_ascii=False, indent=2))
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
