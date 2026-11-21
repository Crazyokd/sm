"""实时只读侧栏；读取落盘摘要，车辆控制留在聊天外。"""
import curses
import datetime
import json
from pathlib import Path
import sys
import time
import unicodedata

ACTIVE = Path(__file__).resolve().parent / 'active.json'

def number(value, digits=2):
    return '--' if value is None else f'{value:.{digits}f}'

def lines_for(record, now):
    if record.get('mode') == 'status_error':
        return [('蓝牙 · 双车实时监控', 'title'), ('本地状态读取失败，采样状态未确认', 'warn')]
    if record.get('mode') in ['cleanup_pending', 'stopping']:
        return [('蓝牙 · 双车实时监控', 'title'), ('采样收尾尚未确认', 'warn'), ('保留会话，停止确认后才允许再次开始', 'muted')]
    if record.get('mode') == 'starting':
        return [('蓝牙 · 双车实时监控', 'title'), ('正在核对目标并启动采样', 'muted'), ('数据就绪后每 5 秒刷新', 'muted')]
    if record.get('mode') == 'waiting_for_vehicles':
        return [('蓝牙 · 双车实时监控', 'title'), ('等待车辆 · 未连接', 'muted'), ('本地监控程序已就绪', 'good'), ('', ''),
                ('WX1   等待车辆地址与身份', 'title'), ('WX2   等待车辆地址与身份', 'title'), ('', ''),
                ('准备后每 5 秒显示', 'muted'), ('中心距离、地图、位姿与定位置信度', 'normal'), ('双向新增帧、累计帧和断流间隔', 'normal'), ('', ''),
                ('记录内容', 'muted'), ('逐帧事实、原始时间戳、位姿和速度', 'normal'), ('蓝牙 HCI、连接状态与缓存 RSSI', 'normal'), ('CPU、内存与采样错误', 'normal'), ('', ''),
                ('当前会话待机，尚未启动采样', 'good'), ('车辆准备好后再开始记录', 'muted')]
    summary = record.get('summary', {})
    age = (now - record.get('epoch_ns', 0)) / 1e9
    data = summary.get('vehicles', {})
    finished = record.get('mode') == 'stopped' or (bool(data) and all(v.get('finished') for v in data.values()))
    stale = age > 12
    errors = any(value.get('error') for value in data.values())
    status = '已停止' if finished else ('反馈异常' if errors else ('反馈延迟' if stale else '正在记录'))
    lines = [('蓝牙 · 双车实时记录', 'title'), (status + '   每 5 秒更新', 'warn' if stale else 'good'),
             ('数据时间  ' + summary.get('local_time', '--'), 'muted'), ('', '')]
    valid = summary.get('distance_valid') and not errors and (finished or not stale)
    lines += [('最后距离' if finished else ('车辆中心估计距离' if summary.get('distance_estimated') else '车辆中心距离'), 'muted'), (number(summary.get('distance_m') if valid else None, 3) + ' m', 'big'),
              ('定位置信度偏低，距离仅作估计' if valid and summary.get('distance_estimated') else ('' if valid else '等待两车同图、有效且新鲜的定位'), 'warn'), ('', '')]
    for alias, peer in [('wx1', 'wx2'), ('wx2', 'wx1')]:
        value = data.get(alias, {})
        pose = value.get('pose') or {}
        lines.append((alias.upper() + ' [' + str(value.get('own_id', '--')) + ']  ←  ' + peer.upper(), 'title'))
        if value.get('error'):
            lines += [('此车采样反馈暂不可用', 'warn'), ('', '')]
            continue
        delta = value.get('peer_frames_delta')
        total = value.get('peer_frames_total')
        lines.append((f'最近 5 秒  {number(delta, 0)} 帧    累计 {number(total, 0)}', 'good' if delta else 'warn'))
        gap = value.get('no_peer_for_s')
        gap_label = '历史收包间隔  ' if stale or finished else '采样时距上帧  '
        current_gap = gap
        lines.append((gap_label + ('尚未收到完整帧' if current_gap is None else number(current_gap, 1) + ' 秒'), 'warn' if current_gap is None or current_gap > 3 else 'normal'))
        if pose and value.get('pose_valid') and not stale:
            lines += [(f'地图 {pose.get("map_id", "--")}  定位置信度 {pose.get("confidence") if pose.get("confidence") is not None else chr(0x672a) + chr(0x77e5)}', 'warn' if value.get('low_confidence') else 'normal'),
                      ('x ' + number(pose.get('x') / 1000 if pose.get('x') is not None else None, 3) + ' m   y ' + number(pose.get('y') / 1000 if pose.get('y') is not None else None, 3) + ' m', 'normal'),
                      ('朝向 ' + number(pose.get('theta') / 1000 if pose.get('theta') is not None else None, 3) + ' rad', 'normal')]
        else:
            lines.append(('位姿未就绪或采样已过期', 'warn'))
        lines += [('本车事实累计  ' + number(value.get('self_frames'), 0) + ' 帧', 'muted'), ('', '')]
    lines += [('原始帧与位姿按车保存，缺口单独标记', 'muted'), ('结束时在聊天里说“停止记录”', 'muted')]
    return lines

def clipped(text, width):
    out = ''
    used = 0
    for char in text:
        size = 2 if unicodedata.east_asian_width(char) in 'WF' else 1
        if used + size > width:
            break
        out += char
        used += size
    return out

def current_record():
    try:
        session = json.loads(ACTIVE.read_text())
        if session.get('monitor_mode') in ['waiting_for_vehicles', 'starting', 'cleanup_pending', 'stopping']:
            return {'mode': session['monitor_mode']}
        record = json.loads((Path(session['local_directory']) / 'latest.json').read_text())
        if session.get('monitor_mode') == 'stopped':
            record['mode'] = 'stopped'
        return record
    except FileNotFoundError:
        return {'mode': 'waiting_for_vehicles' if not ACTIVE.exists() else 'status_error'}
    except (OSError, ValueError, KeyError):
        return {'mode': 'status_error'}


def main(screen):
    curses.curs_set(0)
    curses.use_default_colors()
    for index, color in [(1, curses.COLOR_CYAN), (2, curses.COLOR_GREEN), (3, curses.COLOR_YELLOW)]:
        curses.init_pair(index, color, -1)
    styles = {'title': curses.A_BOLD | curses.color_pair(1), 'big': curses.A_BOLD | curses.color_pair(1),
              'good': curses.color_pair(2), 'warn': curses.color_pair(3), 'muted': curses.A_DIM, 'normal': 0, '': 0}
    screen.timeout(500)
    while True:
        rows = lines_for(current_record(), time.time_ns())
        screen.erase()
        height, width = screen.getmaxyx()
        for y, (text, style) in enumerate(rows[:height-1]):
            try:
                screen.addstr(y, 1, clipped(text, max(0, width-3)), styles.get(style, 0))
            except curses.error:
                pass
        screen.refresh()
        if screen.getch() in [ord('q'), 3]:
            break

if __name__ == '__main__':
    if sys.argv[1:] == ['--snapshot']:
        print('\n'.join(text for text, _ in lines_for(current_record(), time.time_ns())))
    elif not sys.argv[1:]:
        curses.wrapper(main)
    else:
        raise SystemExit('Usage: python3 sidebar.py [--snapshot]')
