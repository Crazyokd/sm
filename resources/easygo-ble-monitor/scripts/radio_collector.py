"""只读采集蓝牙 HCI、连接状态、缓存 RSSI 和主机资源；不调用连接或配置方法。"""
import dbus
import json
from pathlib import Path
import signal
import subprocess
import sys
import time

root = Path(sys.argv[1])
peer_mac = sys.argv[2]
duration = float(sys.argv[3])
root.mkdir(parents=True, exist_ok=True)
stopped = False

def request_stop(*_):
    global stopped
    stopped = True

for sig in [signal.SIGTERM, signal.SIGINT, signal.SIGHUP]:
    signal.signal(sig, request_stop)
children = []
profile = None
files = []
if (root / 'STOP').exists():
    (root / 'finished.json').write_text(json.dumps({'epoch_ns': time.time_ns(), 'finished': True}))
    raise SystemExit(0)
try:
    text = (root / 'hci.txt').open('w')
    files.append(text)
    hci = subprocess.Popen(['btmon', '-i', '0', '-T', '-w', str(root / 'hci.btsnoop')], stdout=text, stderr=subprocess.STDOUT)
    children.append(hci)
    resources = (root / 'resources.jsonl').open('w')
    files.append(resources)
    resource_errors = (root / 'resources.stderr').open('w')
    files.append(resource_errors)
    profile = subprocess.Popen(['/usr/bin/python3', '-u', str(root / 'resource_probe.py'), str(int(duration))], stdout=resources, stderr=resource_errors)
    children.append(profile)
    bus = dbus.SystemBus()
    path = '/org/bluez/hci0/dev_' + peer_mac.replace(':', '_')
    start = time.monotonic()
    with (root / 'radio.jsonl').open('w') as output:
        while not stopped and not (root / 'STOP').exists() and time.monotonic() - start < duration:
            row = {'epoch_ns': time.time_ns(), 'elapsed_s': time.monotonic() - start, 'expected_peer_address': peer_mac,
                   'hci_process_alive': hci.poll() is None, 'resource_process_alive': profile.poll() is None}
            try:
                properties = dbus.Interface(bus.get_object('org.bluez', path, introspect=False), 'org.freedesktop.DBus.Properties').GetAll('org.bluez.Device1', timeout=2)
                row.update(connected=bool(properties.get('Connected')), services_resolved=bool(properties.get('ServicesResolved')),
                           rssi_dbm_cached=int(properties['RSSI']) if 'RSSI' in properties else None)
            except Exception as error:
                row.update(connected=None, services_resolved=None, rssi_dbm_cached=None, error=str(error))
            output.write(json.dumps(row) + '\n')
            output.flush()
            temporary = root / 'status.tmp'
            temporary.write_text(json.dumps(row))
            temporary.replace(root / 'status.json')
            time.sleep(1)
finally:
    for child in children:
        if child.poll() is None:
            child.send_signal(signal.SIGTERM if child is profile else signal.SIGINT)
    for child in children:
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.terminate()
            child.wait(timeout=3)
    for file in files:
        file.close()
    (root / 'finished.json').write_text(json.dumps({'epoch_ns': time.time_ns(), 'finished': True}))
