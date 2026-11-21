"""Analyze a stopped BLE recording offline, using only the Python standard library."""
import argparse
from collections import Counter, defaultdict
import csv
import datetime
import json
import math
from pathlib import Path, PurePosixPath
import statistics
import tarfile


LABELS = ('wx1', 'wx2')
COUNTER_RECEIVERS = {'wx1_to_wx2': 'wx2', 'wx2_to_wx1': 'wx1'}
LIMITATIONS = [
    'Source ages are RAW/uncalibrated, signed receiver wall time minus the matched original sender sample time. '
    'They include sample publication/transport/collection delay and clock offset; corrected one-way latency is unavailable.',
    'Receiver interarrival and dropout use that receiver\'s monotonic clock and only its observed recording window. '
    'Dropout includes the leading and trailing silence; missing recording periods are unknown.',
    'Distance is recomputed only from paired feedback snapshots with same-map, localized, fresh poses. '
    'Polling pairs are not synchronized pose measurements; low or unknown confidence is marked estimated.',
    'Distance bins count polling snapshots, not packet delivery probability or maximum stable range. '
    'Cached RSSI, isolated receptions, telemetry and lack of collision do not prove avoidance benefit. '
    'No vehicle motion or collision analysis is performed.',
    'Resource CPU is percent of one core (may exceed 100%); RSS is MiB (1024**2 bytes). '
    'Process-name summaries pool observed processes; they are not whole-host utilization.',
    'Radio child health is observed at polling times. A finished radio loop alone does not prove uninterrupted '
    'HCI/resource capture; dead children and polling errors mark coverage partial while valid measurements are retained.',
    'Source capture_complete describes recorded stream identity, end evidence and terminal counter consistency. '
    'Recorded invalid/API errors and rejected measurements are separate data-quality gaps; API errors do not prove BLE packet loss. '
    'HCI validation checks only the 16-byte btsnoop header and basic file size, not every packet or capture continuity.',
]


def numeric(value):
    try:
        return type(value) in (int, float) and math.isfinite(value)
    except OverflowError:
        return False


def stats(values):
    values = sorted(value for value in values if numeric(value))
    return {'count': len(values), 'min': values[0] if values else None,
            'mean': statistics.fmean(values) if values else None,
            'p50': statistics.median(values) if values else None,
            'p95': values[math.ceil(len(values) * .95) - 1] if values else None,
            'max': values[-1] if values else None,
            'negative_count': sum(value < 0 for value in values)}


def read_file(path, issues, required=False):
    try:
        return path.read_bytes()
    except FileNotFoundError:
        if required:
            issues.append(f'{path.name}: missing input')
    except OSError as error:
        issues.append(f'{path.name}: cannot read: {error}')
    return None


def object_json(raw, name, issues):
    if raw is None:
        return None
    try:
        result = json.loads(raw)
        if not isinstance(result, dict):
            raise ValueError('expected object')
        return result
    except (ValueError, UnicodeError) as error:
        issues.append(f'{name}: invalid JSON: {error}')
        return None


def json_rows(raw, name, issues):
    if raw is None:
        return []
    rows = []
    for line_number, line in enumerate(raw.splitlines(), 1):
        if not line.strip():
            continue
        row = object_json(line, f'{name}:{line_number}', issues)
        if row is not None:
            rows.append(row)
    return rows


def hci_header(stream, size):
    header = stream.read(16)
    header_valid = len(header) == 16 and header[:8] == b'btsnoop\0' and int.from_bytes(header[8:12], 'big') == 1
    size_valid = size == 16 or size >= 40
    return {'size_bytes': size, 'header_valid': header_valid, 'size_valid': size_valid,
            'valid': header_valid and size_valid, 'validation': 'basic_header_and_size_only'}


def bundle(root, alias, kind, issues):
    """Read only known regular members in memory; never extract archive paths."""
    archive_path = root / f'{alias}-{kind}.tar.gz'
    names = {'status.json', 'launch.json', 'finished.json'}
    names.update({'facts.jsonl'} if kind == 'source' else {'radio.jsonl', 'resources.jsonl', 'hci.btsnoop'})
    contents = {}
    before = len(issues)
    notes = []
    if archive_path.exists():
        origin = 'archive:' + archive_path.name
        try:
            with tarfile.open(archive_path, 'r:gz') as archive:
                for member in archive:
                    path = PurePosixPath(member.name)
                    if path.is_absolute() or '..' in path.parts or member.issym() or member.islnk():
                        issues.append(f'{archive_path.name}: unsafe archive member {member.name!r} ignored')
                        continue
                    if str(path) not in names:
                        continue
                    if not member.isfile() or str(path) in contents:
                        issues.append(f'{archive_path.name}: invalid/duplicate member {member.name!r}')
                        contents[str(path)] = None
                        continue
                    with archive.extractfile(member) as stream:
                        contents[str(path)] = hci_header(stream, member.size) if str(path) == 'hci.btsnoop' else stream.read()
        except (OSError, tarfile.TarError, EOFError) as error:
            issues.append(f'{archive_path.name}: invalid/incomplete archive: {error}')
        if kind == 'source':
            mirror = root / f'{alias}-facts.jsonl'
            if mirror.exists() and 'facts.jsonl' in contents:
                raw = read_file(mirror, notes)
                if raw != contents['facts.jsonl']:
                    notes.append(f'{mirror.name}: local mirror differs; canonical archive selected')
    else:
        origin = 'local_mirror'
        for name in names:
            local_name = f'{alias}-{name}'
            if name == 'status.json' and kind == 'radio':
                local_name = f'{alias}-radio-status.json'
            if name == 'hci.btsnoop':
                path = root / local_name
                try:
                    with path.open('rb') as stream:
                        contents[name] = hci_header(stream, path.stat().st_size)
                except FileNotFoundError:
                    pass
                except OSError as error:
                    issues.append(f'{local_name}: cannot read: {error}')
                continue
            raw = read_file(root / local_name, issues)
            if raw is not None:
                contents[name] = raw
    required = ['facts.jsonl'] if kind == 'source' else ['radio.jsonl', 'resources.jsonl', 'hci.btsnoop']
    for name in required:
        if contents.get(name) is None:
            issues.append(f'{alias}/{kind}/{name}: missing input')
    return contents, {'origin': origin, 'complete': False, 'read_errors': len(issues) - before, 'notes': notes}


def identity(record, target, launch=False):
    keys = ('own_id', 'peer_id', 'mac') if launch else ('own_id', 'expected_peer', 'own_mac')
    return all(record.get(key) == target[field] for key, field in zip(keys, ('own_id', 'peer_id', 'mac')))


def source_data(contents, metadata, alias, target, issues):
    prefix = f'{alias}/source'
    before = len(issues)
    status = object_json(contents.get('status.json'), prefix + '/status.json', issues)
    launch = object_json(contents.get('launch.json'), prefix + '/launch.json', issues)
    rows = json_rows(contents.get('facts.jsonl'), prefix + '/facts.jsonl', issues)
    for record, is_launch in [(status, False), (launch, True)]:
        if record is not None and not identity(record, target, is_launch):
            issues.append(f'{prefix}: archive/status identity mismatch; facts excluded')
            rows = []
    valid, captured, rejected = [], [], Counter()
    capture_complete = metadata['read_errors'] == 0 and len(issues) == before
    for row in rows:
        if row.get('own_id') != target['own_id']:
            rejected['collector identity mismatch'] += 1
            capture_complete = False
            continue
        if not all(type(row.get(key)) is int and 0 <= row[key] < 2 ** 64 for key in ('epoch_ns', 'mono_ns')):
            rejected['invalid collector timestamp'] += 1
            capture_complete = False
            continue
        captured.append(row)
        if row.get('kind') in ('self', 'peer'):
            data = row.get('data')
            geometry = data
            if row['kind'] == 'peer':
                if (not isinstance(data, dict) or not isinstance(data.get('robot_id'), str)
                        or not data['robot_id'] or row.get('robot_id') != data['robot_id']):
                    rejected['fact robot identity mismatch'] += 1
                    capture_complete = False
                    continue
                geometry = data.get('geometry')
            if (not isinstance(geometry, dict) or type(geometry.get('sampled_at_ns')) is not int
                    or not 0 <= geometry['sampled_at_ns'] < 2 ** 63):
                rejected['invalid sample timestamp/geometry'] += 1
                continue
            if (not isinstance(geometry.get('map_id'), str) or not geometry['map_id'] or any(
                    not isinstance(geometry.get(key), list) or len(geometry[key]) != 3
                    or not all(numeric(value) for value in geometry[key]) for key in ('pose', 'velocity'))):
                rejected['incomplete/invalid LocalGeometry map_id, pose or velocity'] += 1
                continue
            footprint = geometry.get('footprint')
            if (not isinstance(footprint, list) or len(footprint) < 3 or any(
                    not isinstance(point, list) or len(point) != 2 or not all(numeric(value) for value in point)
                    for point in footprint)):
                rejected['incomplete/invalid LocalGeometry footprint'] += 1
                continue
        valid.append(row)
    for reason, count in rejected.items():
        issues.append(f'{prefix}: {count} rejected rows: {reason}')
    finished = bool(captured and captured[-1].get('kind') == 'finished')
    if not finished or (status is not None and status.get('finished') is not True):
        issues.append(f'{prefix}: incomplete recording (finished evidence missing)')
        capture_complete = False
    actual = Counter(row['kind'] for row in captured if row.get('kind') in ('self', 'invalid'))
    actual.update('peer:' + row['robot_id'] for row in captured
                  if row.get('kind') == 'peer' and isinstance(row.get('robot_id'), str))
    for label, record in [('status', status), ('finished', captured[-1] if finished else None)]:
        if record is not None and isinstance(record.get('counts'), dict):
            keys = {'self', 'invalid'} | set(actual) | {key for key in record['counts'] if key.startswith('peer:')}
            for key in sorted(keys):
                count = record['counts'].get(key, 0)
                if type(count) is not int or count < 0 or count != actual[key]:
                    issues.append(f'{prefix}: {label} count mismatch for {key}; data incomplete')
                    capture_complete = False
    sampling_errors = {kind: sum(row.get('kind') == kind for row in captured) for kind in ('invalid', 'api_error')}
    for kind, count in sampling_errors.items():
        if count:
            issues.append(f'{prefix}: sampling error {kind}: {count} events; data quality degraded')
    quality = 'degraded' if any(sampling_errors.values()) or rejected else (
        'no_recorded_errors' if capture_complete else 'unknown')
    peers = Counter(row['robot_id'] for row in valid if row.get('kind') == 'peer')
    metadata.update(complete=capture_complete and quality == 'no_recorded_errors', capture_complete=capture_complete,
                    data_quality={'status': quality, 'sampling_error_counts': sampling_errors,
                                  'rejected_rows_by_reason': dict(rejected)},
                    valid_rows=len(valid), rejected_rows=sum(rejected.values()), collector_finished=finished,
                    peer_events_by_robot=dict(peers),
                    other_peer_events_by_robot={rid: count for rid, count in peers.items() if rid != target['peer_id']})
    return valid


def direction(sender, receiver, records, targets):
    sent = [row for row in records[sender] if row.get('kind') == 'self']
    received = [row for row in records[receiver] if row.get('kind') == 'peer'
                and row['robot_id'] == targets[receiver]['peer_id']]
    samples = {row['data']['sampled_at_ns'] for row in sent}
    matched = [row for row in received if row['data']['geometry']['sampled_at_ns'] in samples]
    times = sorted(row['mono_ns'] for row in received if type(row.get('mono_ns')) is int)
    window = [row['mono_ns'] for row in records[receiver] if type(row.get('mono_ns')) is int]
    gaps = [(right - left) / 1e9 for left, right in zip(times, times[1:])]
    duration = (max(window) - min(window)) / 1e9 if len(window) >= 2 else None
    dropout = None
    if duration is not None:
        anchors = [min(window)] + times + [max(window)]
        dropout = max((right - left) / 1e9 for left, right in zip(anchors, anchors[1:]))
    return {
        'sender_self_events': len(sent), 'sender_unique_samples': len(samples),
        'receiver_peer_events': len(received),
        'receiver_unique_samples': len({row['data']['geometry']['sampled_at_ns'] for row in received}),
        'matched_receiver_events': len(matched), 'unmatched_receiver_events': len(received) - len(matched),
        'receiver_interarrival_s': stats(gaps), 'receiver_observation_duration_s': duration,
        'max_dropout_s': dropout,
        'raw_source_age_ms': stats((row['epoch_ns'] - row['data']['geometry']['sampled_at_ns']) / 1e6
                                   for row in matched if type(row.get('epoch_ns')) is int),
        'source_age_basis': 'RAW/uncalibrated; receiver epoch_ns minus matched sender sampled_at_ns; '
                            'self/peer matching uses original sampled_at_ns only',
        'corrected_one_way_latency_ms': None,
    }


def resource_summary(rows, issues, alias):
    grouped = defaultdict(lambda: {'cpu': [], 'rss': []})
    missing = Counter()
    total_cpu, total_rss = [], []
    for row in rows:
        processes = row.get('processes')
        if not isinstance(processes, list):
            issues.append(f'{alias}/resources: invalid processes list')
            continue
        unavailable = row.get('unavailable', [])
        if isinstance(unavailable, list):
            missing.update(name for name in unavailable if isinstance(name, str))
        snapshot_cpu, snapshot_rss = [], []
        for process in processes:
            if not isinstance(process, dict) or not isinstance(process.get('name'), str):
                issues.append(f'{alias}/resources: invalid process record')
                continue
            values = grouped[process['name']]
            cpu, rss = process.get('cpu_percent_one_core'), process.get('rss_bytes')
            if numeric(cpu) and cpu >= 0:
                values['cpu'].append(cpu)
                snapshot_cpu.append(cpu)
            elif cpu is not None:
                issues.append(f'{alias}/resources: invalid CPU value')
            if numeric(rss) and rss >= 0:
                values['rss'].append(rss / 1024 ** 2)
                snapshot_rss.append(rss / 1024 ** 2)
            else:
                issues.append(f'{alias}/resources: missing/invalid RSS value')
        if processes and len(snapshot_cpu) == len(processes):
            total_cpu.append(sum(snapshot_cpu))
        if processes and len(snapshot_rss) == len(processes):
            total_rss.append(sum(snapshot_rss))
    return {'samples': len(rows), 'by_process': {
        name: {'cpu_percent_one_core': stats(values['cpu']), 'rss_mib': stats(values['rss'])}
        for name, values in sorted(grouped.items())}, 'unavailable_samples': dict(missing),
        'tracked_process_totals': {'cpu_percent_one_core': stats(total_cpu), 'rss_mib': stats(total_rss)}}


def pose_for_distance(value, target, alias, flags, poll_epoch):
    if not isinstance(value, dict) or not value:
        flags.append(alias + ':missing_input')
        return None
    if not identity(value, target):
        flags.append(alias + ':identity_mismatch')
        return None
    if value.get('error') or value.get('api_error'):
        flags.append(alias + ':input_error')
        return None
    try:
        api = value['vehicle']
        localization = api['status']['localization']
        pose = localization['pose']
        if localization.get('localized') is not True:
            flags.append(alias + ':not_localized')
            return None
        if not isinstance(pose.get('map_id'), str) or not pose['map_id'] or not all(
                numeric(pose.get(key)) for key in ('x', 'y')):
            raise ValueError('map/coordinates missing or invalid')
        epoch = value['epoch_ns']
        if type(epoch) is not int or type(api['epoch_ns']) is not int:
            raise ValueError('invalid timestamp')
        updated = datetime.datetime.fromisoformat(pose['updated_at'].replace('Z', '+00:00'))
        if updated.tzinfo is None:
            raise ValueError('pose timestamp lacks timezone')
        api_age = (epoch - api['epoch_ns']) / 1e9
        pose_age = epoch / 1e9 - updated.timestamp()
        remote_age = value.get('remote_status_age_s')
        query_start, query_end = value.get('query_start_ns'), value.get('query_end_ns')
        if (type(poll_epoch) is not int or type(query_start) is not int or type(query_end) is not int
                or not 0 <= query_end - query_start <= 5e9 or not 0 <= query_start - poll_epoch <= 5e9):
            flags.append(alias + ':missing_or_stale_poll_timing')
            return None
        if (not 0 <= api_age <= 2 or not -.2 <= pose_age <= 2
                or not numeric(remote_age) or not -.2 <= remote_age <= 2):
            flags.append(alias + ':stale_pose_or_status')
            return None
        confidence = pose.get('confidence')
        if not numeric(confidence):
            flags.append(alias + ':unknown_confidence')
        elif confidence < 40:
            flags.append(alias + ':low_confidence')
        return pose
    except (KeyError, TypeError, ValueError, AttributeError, OverflowError):
        flags.append(alias + ':missing_or_invalid_pose_input')
        return None


def distance_timeline(feedback, targets, issues):
    timeline = []
    previous_counts = {}
    bins = defaultdict(lambda: {'snapshots': 0, 'estimated_snapshots': 0, **{
        prefix + suffix: 0 for prefix in COUNTER_RECEIVERS for suffix in
        ('_polls_with_new_frames', '_polls_without_new_frames', '_polls_unknown_delta')}})
    for snapshot in feedback:
        flags = []
        vehicles = snapshot.get('vehicles')
        if not isinstance(vehicles, dict):
            vehicles = {}
        poses = {alias: pose_for_distance(vehicles.get(alias), targets[alias], alias, flags,
                                         snapshot.get('epoch_ns')) for alias in LABELS}
        distance = None
        if all(poses.values()):
            a, b = poses['wx1'], poses['wx2']
            if a['map_id'] != b['map_id']:
                flags.append('map_mismatch')
            else:
                distance = math.hypot(a['x'] - b['x'], a['y'] - b['y']) / 1000
        estimated = any('confidence' in flag for flag in flags)
        counters = {}
        for prefix, receiver in COUNTER_RECEIVERS.items():
            value = vehicles.get(receiver)
            count, delta, gap = None, None, None
            if isinstance(value, dict) and identity(value, targets[receiver]) and not value.get('error'):
                counts = value.get('counts')
                candidate = counts.get('peer:' + targets[receiver]['peer_id']) if isinstance(counts, dict) else None
                if type(candidate) is int and candidate >= 0:
                    count = candidate
                    previous = previous_counts.get(prefix)
                    if previous is not None:
                        if count >= previous:
                            delta = count - previous
                        else:
                            flags.append(prefix + ':counter_reset')
                    previous_counts[prefix] = count
                else:
                    flags.append(prefix + ':missing_or_invalid_counter')
                candidate = value.get('last_peer_gap_s')
                if numeric(candidate) and candidate >= 0:
                    gap = candidate
            counters.update({prefix + '_peer_count': count, prefix + '_peer_delta': delta,
                             prefix + '_observed_gap_s': gap})
        row = {'epoch_ns': snapshot.get('epoch_ns'), 'elapsed_s': snapshot.get('elapsed_s'),
               'distance_m': distance, 'distance_valid': distance is not None,
               'distance_estimated': estimated, 'flags': ';'.join(flags), **counters}
        timeline.append(row)
        if distance is not None:
            bucket = bins[math.floor(distance)]
            bucket['snapshots'] += 1
            bucket['estimated_snapshots'] += int(estimated)
            for prefix in COUNTER_RECEIVERS:
                delta = counters[prefix + '_peer_delta']
                suffix = '_polls_unknown_delta' if delta is None else (
                    '_polls_with_new_frames' if delta > 0 else '_polls_without_new_frames')
                bucket[prefix + suffix] += 1
    if not feedback:
        issues.append('feedback.jsonl: no paired evidence; distance unavailable')
    elif not any(row['distance_valid'] for row in timeline):
        issues.append('feedback.jsonl: no valid paired distance; see timeline flags')
    return timeline, [{'lower_m': key, 'upper_m_exclusive': key + 1, **value} for key, value in sorted(bins.items())]


def write_csv(path, fields, rows):
    with path.open('w', newline='') as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)


def analyze(session_dir, output_dir=None):
    root = Path(session_dir)
    output = Path(output_dir) if output_dir is not None else root / 'analysis'
    if output.resolve().is_relative_to(Path(__file__).resolve().parents[1]):
        raise ValueError('Analysis output must be outside the installed Skill directory')
    issues = []
    session = object_json(read_file(root / 'session.json', issues, required=True), 'session.json', issues) or {}
    targets = session.get('targets')
    valid_targets = isinstance(targets, dict) and all(
        isinstance(targets.get(alias), dict) and all(isinstance(targets[alias].get(key), str) and targets[alias][key]
            for key in ('own_id', 'peer_id', 'mac', 'host')) for alias in LABELS)
    if valid_targets:
        valid_targets = all(targets['wx1'][key].upper() != targets['wx2'][key].upper()
                            for key in ('own_id', 'mac', 'host')) and all(
            targets[a]['peer_id'] == targets[b]['own_id'] for a, b in [('wx1', 'wx2'), ('wx2', 'wx1')])
    if not valid_targets:
        issues.append('session.json: missing/invalid two-vehicle identities; data attribution unavailable')
        targets = {}
    feedback = json_rows(read_file(root / 'feedback.jsonl', issues, required=True), 'feedback.jsonl', issues)
    records, inputs, vehicles = {}, {}, {}
    for alias in LABELS if targets else []:
        source, source_meta = bundle(root, alias, 'source', issues)
        records[alias] = source_data(source, source_meta, alias, targets[alias], issues)
        radio, radio_meta = bundle(root, alias, 'radio', issues)
        radio_before = len(issues)
        launch = object_json(radio.get('launch.json'), alias + '/radio/launch.json', issues)
        if launch is not None and not identity(launch, targets[alias], launch=True):
            issues.append(alias + '/radio: archive identity mismatch; radio/resources excluded')
            radio = {}
        radio_rows = json_rows(radio.get('radio.jsonl'), alias + '/radio/radio.jsonl', issues)
        peer = 'wx2' if alias == 'wx1' else 'wx1'
        valid_radio = [row for row in radio_rows if isinstance(row.get('expected_peer_address'), str)
                       and row['expected_peer_address'].upper() == targets[peer]['mac'].upper()]
        if len(valid_radio) != len(radio_rows):
            issues.append(alias + '/radio: peer MAC identity mismatch; rows excluded')
        coverage = {}
        for field in ('hci_process_alive', 'resource_process_alive'):
            alive = sum(row.get(field) is True for row in valid_radio)
            dead = sum(row.get(field) is False for row in valid_radio)
            unknown = len(valid_radio) - alive - dead
            state = 'partial' if dead else 'unknown' if unknown or not valid_radio else 'observed_alive'
            coverage[field] = {'status': state, 'alive_samples': alive, 'dead_samples': dead,
                               'unknown_samples': unknown}
            if state != 'observed_alive':
                issues.append(f'{alias}/radio: {field} capture coverage {state}; '
                              f'{dead} dead samples, {unknown} unknown samples')
        poll_errors = sum(bool(row.get('error')) for row in valid_radio)
        coverage['radio_properties'] = {'status': 'partial' if poll_errors else 'observed' if valid_radio else 'unknown',
                                        'error_samples': poll_errors}
        if poll_errors:
            issues.append(f'{alias}/radio: radio_properties capture coverage partial; {poll_errors} polling errors')
        resource_rows = json_rows(radio.get('resources.jsonl'), alias + '/radio/resources.jsonl', issues)
        coverage['resources'] = {'status': 'observed' if resource_rows else 'unavailable', 'samples': len(resource_rows)}
        if not resource_rows:
            issues.append(alias + '/radio/resources.jsonl: empty or unavailable recording; resource coverage unavailable')
        resources = resource_summary(resource_rows, issues, alias)
        hci = radio.get('hci.btsnoop')
        if hci is not None and not hci['valid']:
            issues.append(alias + '/radio/hci.btsnoop: invalid/truncated basic btsnoop header or file size')
        finished = object_json(radio.get('finished.json'), alias + '/radio/finished.json', issues)
        radio_meta.update(complete=bool(finished and finished.get('finished') is True
                                       and radio_meta['read_errors'] == 0 and len(issues) == radio_before),
                          collector_finished=bool(finished and finished.get('finished') is True),
                          capture_coverage=coverage, hci_capture=hci,
                          radio_rows=len(valid_radio), resource_rows=len(resource_rows))
        if not radio_meta['complete']:
            issues.append(alias + '/radio: incomplete recording (missing finished evidence or partial/unknown capture coverage)')
        inputs[alias] = {'source': source_meta, 'radio': radio_meta}
        vehicles[alias] = {'resources': resources,
                           'radio': {'samples': len(valid_radio), 'rssi_dbm_cached': stats(
                               row.get('rssi_dbm_cached') for row in valid_radio),
                                     'rssi_basis': 'BlueZ cached RSSI; not per-packet RSSI or proof of current link quality'}}
    directions = {f'{sender}->{receiver}': direction(sender, receiver, records, targets)
                  for sender, receiver in [('wx1', 'wx2'), ('wx2', 'wx1')]} if targets else {}
    timeline, bins = distance_timeline(feedback, targets, issues) if targets else ([], [])
    if (root / 'clock_calibration.json').exists():
        issues.append('clock_calibration.json: correction not applied; all source ages remain RAW/uncalibrated')
    summary = {'data_status': 'incomplete' if issues else 'complete', 'inputs': inputs,
               'as_started_targets': {alias: {'transport': target.get('transport'),
                                             'package_versions': target.get('package_versions')}
                                      for alias, target in targets.items()},
               'directions': directions, 'vehicles': vehicles,
               'distance': {'paired_snapshots': len(timeline),
                            'valid_snapshots': sum(row['distance_valid'] for row in timeline),
                            'estimated_snapshots': sum(row['distance_valid'] and row['distance_estimated'] for row in timeline),
                            'association': 'paired feedback snapshot only; polling is not synchronized',
                            'bin_basis': 'Counts are preceding poll-window observations since the previous valid counter snapshot, '
                                         'assigned to the current snapshot distance. They may span missing polls; '
                                         'they are not packet delivery probability or evidence of maximum stable range.'},
               'issues': issues, 'notes': [note for evidence in inputs.values() for metadata in evidence.values()
                                          for note in metadata['notes']], 'limitations': LIMITATIONS}
    output.mkdir(parents=True, exist_ok=True)
    (output / 'summary.json').write_text(json.dumps(summary, indent=2, ensure_ascii=False, allow_nan=False) + '\n')
    counter_fields = [prefix + suffix for prefix in COUNTER_RECEIVERS
                      for suffix in ('_peer_count', '_peer_delta', '_observed_gap_s')]
    bin_fields = [prefix + suffix for prefix in COUNTER_RECEIVERS
                  for suffix in ('_polls_with_new_frames', '_polls_without_new_frames', '_polls_unknown_delta')]
    write_csv(output / 'timeline.csv', ['epoch_ns', 'elapsed_s', 'distance_m', 'distance_valid', 'distance_estimated', 'flags']
              + counter_fields, timeline)
    write_csv(output / 'distance-bins.csv', ['lower_m', 'upper_m_exclusive', 'snapshots', 'estimated_snapshots']
              + bin_fields, bins)
    report = ['# BLE offline recording analysis', '', f'Data status: **{summary["data_status"]}**.', '',
              '| Direction | Sender self | Receiver peer | Matched sample events | Max interarrival (s) | Max dropout (s) | RAW age min / mean / max (ms) |',
              '| --- | ---: | ---: | ---: | ---: | ---: | --- |']
    for name, item in directions.items():
        age = item['raw_source_age_ms']
        report.append(f'| {name} | {item["sender_self_events"]} | {item["receiver_peer_events"]} | '
                      f'{item["matched_receiver_events"]} | {item["receiver_interarrival_s"]["max"]} | '
                      f'{item["max_dropout_s"]} | {age["min"]} / {age["mean"]} / {age["max"]} |')
    other_peers = [(alias, rid, count) for alias, evidence in inputs.items()
                   for rid, count in evidence['source']['other_peer_events_by_robot'].items()]
    if other_peers:
        report.extend(['', 'Other recorded peers, excluded from selected-pair metrics:', '',
                       '| Receiver | Other robot ID | Peer events |', '| --- | --- | ---: |'])
        report.extend(f'| {alias} | {rid} | {count} |' for alias, rid, count in other_peers)
    report.extend(['', f'Valid paired distance snapshots: {summary["distance"]["valid_snapshots"]}; '
                   f'estimated: {summary["distance"]["estimated_snapshots"]}. Missing values are unavailable.', '',
                   summary['distance']['bin_basis'], '',
                   '| Vehicle / process | CPU mean / max (% of one core) | RSS mean / max (MiB) |',
                   '| --- | ---: | ---: |'])
    for alias, vehicle in vehicles.items():
        processes = {'tracked process total': vehicle['resources']['tracked_process_totals'],
                     **vehicle['resources']['by_process']}
        for name, values in processes.items():
            cpu, rss = values['cpu_percent_one_core'], values['rss_mib']
            report.append(f'| {alias} / {name} | {cpu["mean"]} / {cpu["max"]} | {rss["mean"]} / {rss["max"]} |')
    report.extend(['', '| Source | Capture complete | Data quality | invalid / api_error events | Rejected rows |',
                   '| --- | --- | --- | ---: | ---: |'])
    for alias, evidence in inputs.items():
        source = evidence['source']
        quality = source['data_quality']
        errors = quality['sampling_error_counts']
        report.append(f'| {alias} | {source["capture_complete"]} | {quality["status"]} | '
                      f'{errors["invalid"]} / {errors["api_error"]} | {source["rejected_rows"]} |')
    report.extend(['', '## Evidence limits', ''] + ['- ' + text for text in LIMITATIONS])
    report.extend(['', '## Input issues', ''] + (['- ' + text for text in issues] or ['None detected.']))
    if summary['notes']:
        report.extend(['', '## Input notes', ''] + ['- ' + text for text in summary['notes']])
    (output / 'report.md').write_text('\n'.join(report) + '\n')
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('session_dir', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        summary = analyze(args.session_dir, args.output)
    except ValueError as error:
        parser.error(str(error))
    print(json.dumps({'output': str(args.output or args.session_dir / 'analysis'), 'data_status': summary['data_status'],
                      'issues': len(summary['issues'])}))


if __name__ == '__main__':
    main()
