"""只准备独立本地运行目录；不连接或读取车辆。"""
import argparse
import json
from pathlib import Path
import shutil

SCRIPTS = Path(__file__).resolve().parent
RUNTIME = ('collector.py', 'radio_collector.py', 'resource_probe.py', 'feedback.py',
           'start_recording.py', 'stop_recording.py', 'sidebar.py')


def prepare(workspace):
    root = Path(workspace).resolve()
    if root.is_relative_to(SCRIPTS.parent.resolve()):
        raise ValueError('运行目录必须位于 Skill 安装树之外')
    if root.exists() and any(root.iterdir()):
        raise FileExistsError('运行目录非空；复用已有目录内的程序，或选择新的目录')
    root.mkdir(parents=True, exist_ok=True)
    for name in RUNTIME:
        shutil.copy2(SCRIPTS / name, root / name)
    waiting = root / 'waiting'
    waiting.mkdir()
    state = {'monitor_mode': 'waiting_for_vehicles', 'local_directory': str(waiting), 'targets': {}}
    (root / 'active.json').write_text(json.dumps(state, ensure_ascii=False, indent=2))
    config = {label: {'host': None, 'ssh_user': 'std', 'ssh_port': 22,
                      'robot_id': None, 'bluetooth_mac': None} for label in ('wx1', 'wx2')}
    (root / 'targets.json').write_text(json.dumps(config, ensure_ascii=False, indent=2))
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps({'workspace': str(prepare(args.workspace)), 'mode': 'waiting_for_vehicles'}, ensure_ascii=False))


if __name__ == '__main__':
    main()
