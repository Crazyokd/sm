"""只读记录本车/邻车 ROS 事实、定位和速度；STOP 文件或信号结束。"""
import collections
import fcntl
import json
import math
import os
from pathlib import Path
import signal
import sys
import threading
import time
import urllib.request
import rclpy
from rclpy.signals import SignalHandlerOptions
from std_msgs.msg import String

peer_id, own_id, own_mac, directory = sys.argv[1:5]
root = Path(directory)
root.mkdir(parents=True, exist_ok=True)
lock_file = (root / 'lock').open('w')
fcntl.flock(lock_file, fcntl.LOCK_EX | fcntl.LOCK_NB)
stop = threading.Event()
output_lock = threading.Lock()
counts = collections.Counter({'self': 0, 'peer:' + peer_id: 0, 'invalid': 0})
last = {}
last_peer_mono = None
max_gap = 0.0
vehicle = None
api_error = None
started = time.monotonic()

def emit(kind, **fields):
    row = {'kind': kind, 'epoch_ns': time.time_ns(), 'mono_ns': time.monotonic_ns(), 'own_id': own_id, **fields}
    with output_lock:
        print(json.dumps(row, ensure_ascii=False), flush=True)

def api_watch():
    global vehicle, api_error
    while not stop.is_set():
        try:
            with urllib.request.urlopen('http://127.0.0.1:18088/api/v6/vehicle/status', timeout=2) as response:
                status = json.load(response)['data']
            if status['bluetooth']['address'] != own_mac:
                raise RuntimeError('identity mismatch')
            vehicle = {'epoch_ns': time.time_ns(), 'status': status}
            api_error = None
            emit('vehicle', data=status)
        except Exception as error:
            api_error = str(error)
            emit('api_error', error=str(error))
        stop.wait(0.5)

def validate_geometry(geometry):
    if not isinstance(geometry, dict) or type(geometry.get('sampled_at_ns')) is not int or not 0 <= geometry['sampled_at_ns'] < 2 ** 63:
        raise ValueError('invalid sample timestamp/geometry')
    if not isinstance(geometry.get('map_id'), str) or not geometry['map_id']:
        raise ValueError('invalid map_id')
    vectors = [(geometry.get('pose'), 3), (geometry.get('velocity'), 3)]
    footprint = geometry.get('footprint')
    if not isinstance(footprint, list) or len(footprint) < 3:
        raise ValueError('invalid footprint')
    vectors.extend((point, 2) for point in footprint)
    if any(not isinstance(vector, list) or len(vector) != size or not all(
            type(value) in (int, float) and math.isfinite(value) for value in vector) for vector, size in vectors):
        raise ValueError('invalid geometry vectors')


def receive(kind, message):
    global last_peer_mono, max_gap
    try:
        data = json.loads(message.data)
        if kind == 'self':
            validate_geometry(data)
            counts['self'] += 1
            last['self'] = data
            emit('self', data=data)
            return
        rid = data['robot_id']
        geometry = data['geometry']
        if not isinstance(rid, str) or not rid:
            raise ValueError('invalid peer identity')
        validate_geometry(geometry)
        raw_age = (time.time_ns() - geometry['sampled_at_ns']) / 1e6
        counts['peer:' + rid] += 1
        now = time.monotonic()
        if rid == peer_id:
            if last_peer_mono is not None:
                max_gap = max(max_gap, now - last_peer_mono)
            last_peer_mono = now
            last['peer'] = data
        emit('peer', robot_id=rid, data=data, raw_source_age_ms=raw_age)
    except Exception as error:
        counts['invalid'] += 1
        emit('invalid', error=str(error), raw=message.data)

def save_status(finished=False):
    now = time.monotonic()
    row = {'epoch_ns': time.time_ns(), 'pid': os.getpid(), 'own_id': own_id, 'own_mac': own_mac, 'expected_peer': peer_id,
           'elapsed_s': now - started, 'counts': dict(counts), 'last': dict(last), 'vehicle': vehicle,
           'api_error': api_error, 'last_peer_gap_s': None if last_peer_mono is None else now - last_peer_mono,
           'max_peer_interarrival_s': max_gap, 'finished': finished}
    temp = root / 'status.tmp'
    temp.write_text(json.dumps(row, ensure_ascii=False))
    temp.replace(root / 'status.json')

for sig in [signal.SIGINT, signal.SIGTERM, signal.SIGHUP]:
    signal.signal(sig, lambda *_: stop.set())
if (root / 'STOP').exists():
    emit('finished', counts=dict(counts), reason='stopped_before_start')
    save_status(finished=True)
    raise SystemExit(0)
rclpy.init(signal_handler_options=SignalHandlerOptions.NO)
node = rclpy.create_node('ble_movement_readonly_recorder')
subscriptions = [node.create_subscription(String, topic, lambda message, kind=kind: receive(kind, message), 100)
                 for topic, kind in [('/peer_information/self', 'self'), ('/peer_information/peer', 'peer')]]
thread = threading.Thread(target=api_watch, daemon=True)
thread.start()
emit('started', peer_id=peer_id, pid=os.getpid())
next_status = 0.0
try:
    while not stop.is_set() and not (root / 'STOP').exists() and time.monotonic() - started < 7200:
        rclpy.spin_once(node, timeout_sec=0.1)
        if time.monotonic() >= next_status:
            save_status()
            next_status = time.monotonic() + 0.5
finally:
    stop.set()
    thread.join(timeout=3)
    emit('finished', counts=dict(counts))
    save_status(finished=True)
    node.destroy_node()
    rclpy.shutdown()
