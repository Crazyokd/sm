"""在明确选择的 tmux pane 右侧显示监控；保留聊天焦点。"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

TAG = '@easygo_ble_monitor_workspace'


def tmux(*args):
    return subprocess.run(['tmux', *args], text=True, capture_output=True, check=True).stdout.strip()


def show(workspace, target):
    root = Path(workspace).resolve()
    if not (root / 'sidebar.py').is_file():
        raise ValueError('请先准备监控运行目录')
    metadata = root / 'panel.json'
    if metadata.exists():
        pane = json.loads(metadata.read_text())['pane']
        try:
            if tmux('show-options', '-pv', '-t', pane, TAG) == str(root):
                tmux('respawn-pane', '-k', '-t', pane, sys.executable, str(root / 'sidebar.py'))
                return pane
        except subprocess.CalledProcessError:
            pass
    pane = tmux('split-window', '-h', '-d', '-p', '34', '-t', target,
                '-P', '-F', '#{pane_id}', sys.executable, str(root / 'sidebar.py'))
    tmux('set-option', '-p', '-t', pane, TAG, str(root))
    metadata.write_text(json.dumps({'pane': pane}))
    return pane


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', required=True, type=Path)
    parser.add_argument('--pane-target', required=True)
    args = parser.parse_args()
    print(show(args.workspace, args.pane_target))


if __name__ == '__main__':
    main()
