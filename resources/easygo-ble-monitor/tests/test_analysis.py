"""Offline behavioral checks using anonymous synthetic recordings only."""
import csv
import datetime
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'analyze.py'
EPOCH = 1700000000000000000
BTSNOOP_HEADER = b'btsnoop\0' + (1).to_bytes(4, 'big') + (2001).to_bytes(4, 'big')
TARGETS = {
    'wx1': {'own_id': 'alpha', 'peer_id': 'beta', 'mac': 'AA:00:00:00:00:01', 'host': '192.0.2.1'},
    'wx2': {'own_id': 'beta', 'peer_id': 'alpha', 'mac': 'AA:00:00:00:00:02', 'host': '192.0.2.2'},
}


def fact(kind, alias, seconds, sampled_at_ns=None, robot_id=None):
    row = {'kind': kind, 'own_id': TARGETS[alias]['own_id'],
           'epoch_ns': EPOCH + int(seconds * 1e9), 'mono_ns': int((100 + seconds) * 1e9)}
    if sampled_at_ns is not None:
        rid = robot_id or TARGETS[alias]['own_id' if kind == 'self' else 'peer_id']
        geometry = {'map_id': 'map-a', 'sampled_at_ns': sampled_at_ns, 'pose': [0, 0, 0],
                    'velocity': [0, 0, 0], 'footprint': [[0, 0], [1, 0], [0, 1]]}
        row['data'] = geometry
        if kind == 'peer':
            row['robot_id'] = rid
            row['data'] = {'robot_id': rid, 'instance_id': [0] * 16, 'geometry': geometry}
    return row


def status(alias, counts=None, finished=True):
    target = TARGETS[alias]
    return {'own_id': target['own_id'], 'expected_peer': target['peer_id'], 'own_mac': target['mac'],
            'finished': finished, 'counts': counts or {}}


def snapshot(alias, x, confidence=80, map_id='map-a', seconds=0):
    epoch = EPOCH + int(seconds * 1e9)
    row = status(alias)
    row.update(epoch_ns=epoch, counts={'peer:' + TARGETS[alias]['peer_id']: 10},
               query_start_ns=epoch, query_end_ns=epoch + 100000000,
               remote_status_age_s=0.1, api_error=None, last_peer_gap_s=0.1)
    pose = {'x': x, 'y': 0, 'map_id': map_id, 'confidence': confidence,
            'updated_at': datetime.datetime.fromtimestamp(epoch / 1e9, datetime.timezone.utc).isoformat()}
    row['vehicle'] = {'epoch_ns': epoch, 'status': {'localization': {'localized': True, 'pose': pose}}}
    return row


class AnalysisTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.write_json('session.json', {'targets': TARGETS})

    def write_json(self, name, value):
        (self.root / name).write_text(json.dumps(value))

    def write_rows(self, name, rows):
        (self.root / name).write_text(''.join(json.dumps(row) + '\n' for row in rows))

    def archive(self, alias, kind, members):
        with tarfile.open(self.root / f'{alias}-{kind}.tar.gz', 'w:gz') as archive:
            for name, value in members.items():
                raw = value if isinstance(value, bytes) else json.dumps(value).encode()
                member = tarfile.TarInfo(name)
                member.size = len(raw)
                archive.addfile(member, io.BytesIO(raw))

    def analyze(self, output=None):
        args = [sys.executable, str(SCRIPT), str(self.root)]
        if output:
            args.extend(['--output', str(output)])
        run = subprocess.run(args, capture_output=True, text=True, timeout=10)
        self.assertEqual(run.returncode, 0, run.stderr)
        destination = output or self.root / 'analysis'
        for name in ['summary.json', 'report.md', 'timeline.csv', 'distance-bins.csv']:
            self.assertTrue((destination / name).is_file(), name)
        return json.loads((destination / 'summary.json').read_text())

    def test_missing_inputs_leave_metrics_unavailable(self):
        summary = self.analyze(self.root / 'custom')
        direction = summary['directions']['wx1->wx2']
        self.assertEqual(direction['receiver_peer_events'], 0)
        self.assertIsNone(direction['max_dropout_s'])
        self.assertIsNone(direction['raw_source_age_ms']['mean'])
        self.assertEqual(summary['distance']['valid_snapshots'], 0)
        self.assertTrue(summary['issues'])

    def test_copied_analyzer_rejects_outputs_inside_skill_tree_and_allows_external_output(self):
        skill = self.root / 'fake-skill'
        script = skill / 'scripts' / 'analyze.py'
        script.parent.mkdir(parents=True)
        shutil.copyfile(SCRIPT, script)
        session = skill / 'recordings' / 'sample'
        session.mkdir(parents=True)
        (session / 'session.json').write_text(json.dumps({'targets': TARGETS}))
        redirect = self.root / 'redirect'
        redirect.symlink_to(skill / 'protected-output', target_is_directory=True)
        for args, destination in [([], session / 'analysis'),
                                  (['--output', str(skill / 'custom-output')], skill / 'custom-output'),
                                  (['--output', str(redirect)], skill / 'protected-output')]:
            with self.subTest(destination=destination):
                run = subprocess.run([sys.executable, str(script), str(session)] + args,
                                     capture_output=True, text=True, timeout=10)
                self.assertNotEqual(run.returncode, 0)
                self.assertIn('outside', run.stderr)
                self.assertFalse(destination.exists())
        external = self.root / 'external-analysis'
        run = subprocess.run([sys.executable, str(script), str(session), '--output', str(external)],
                             capture_output=True, text=True, timeout=10)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertTrue((external / 'summary.json').is_file())

    def test_completed_zero_reception_window_reports_full_dropout(self):
        rows = [fact('started', 'wx2', 0), fact('finished', 'wx2', 8)]
        self.write_rows('wx2-facts.jsonl', rows)
        summary = self.analyze()
        self.assertEqual(summary['directions']['wx1->wx2']['max_dropout_s'], 8)

    def test_clock_skew_preserves_negative_age_and_matches_original_samples(self):
        sampled = EPOCH + 1000000000
        self.write_rows('wx1-facts.jsonl', [fact('self', 'wx1', 0.1, sampled)])
        self.write_rows('wx2-facts.jsonl', [fact('peer', 'wx2', 0.5, sampled),
                                         fact('peer', 'wx2', 0.6, sampled + 1)])
        summary = self.analyze()
        direction = summary['directions']['wx1->wx2']
        self.assertEqual(direction['matched_receiver_events'], 1)
        self.assertEqual(direction['unmatched_receiver_events'], 1)
        self.assertEqual(direction['raw_source_age_ms']['min'], -500)
        self.assertEqual(direction['raw_source_age_ms']['negative_count'], 1)
        self.assertIsNone(direction['corrected_one_way_latency_ms'])
        self.assertIn('uncalibrated', direction['source_age_basis'])

    def test_flat_self_geometry_matches_wrapped_peer_by_original_sample_time(self):
        self.write_rows('wx1-facts.jsonl', [{
            'kind': 'self', 'own_id': 'alpha', 'epoch_ns': EPOCH + 100000000, 'mono_ns': 100100000000,
            'data': {'map_id': 'map-a', 'sampled_at_ns': EPOCH, 'pose': [1000, 2000, 0],
                     'velocity': [0, 0, 0], 'footprint': [[0, 0], [1, 0], [0, 1]]},
        }])
        self.write_rows('wx2-facts.jsonl', [{
            'kind': 'peer', 'own_id': 'beta', 'robot_id': 'alpha',
            'epoch_ns': EPOCH + 250000000, 'mono_ns': 100250000000,
            'data': {'robot_id': 'alpha', 'instance_id': [0] * 16,
                     'geometry': {'map_id': 'map-a', 'sampled_at_ns': EPOCH, 'pose': [1000, 2000, 0],
                                  'velocity': [0, 0, 0], 'footprint': [[0, 0], [1, 0], [0, 1]]}},
        }])
        direction = self.analyze()['directions']['wx1->wx2']
        self.assertEqual(direction['sender_self_events'], 1)
        self.assertEqual(direction['sender_unique_samples'], 1)
        self.assertEqual(direction['receiver_peer_events'], 1)
        self.assertEqual(direction['matched_receiver_events'], 1)
        self.assertEqual(direction['unmatched_receiver_events'], 0)
        self.assertEqual(direction['raw_source_age_ms']['mean'], 250)

    def test_interarrival_uses_receiver_monotonic_clock_and_includes_tail(self):
        rows = [fact('started', 'wx2', 0), fact('peer', 'wx2', 1, EPOCH),
                fact('peer', 'wx2', 3, EPOCH + 1), fact('finished', 'wx2', 9)]
        rows[2]['epoch_ns'] = EPOCH - 10000000000
        self.write_rows('wx2-facts.jsonl', rows)
        direction = self.analyze()['directions']['wx1->wx2']
        self.assertEqual(direction['receiver_interarrival_s']['max'], 2)
        self.assertEqual(direction['max_dropout_s'], 6)

    def test_archive_wins_over_partial_local_mirror(self):
        rows = [fact('started', 'wx2', 0), fact('peer', 'wx2', 1, EPOCH),
                fact('peer', 'wx2', 2, EPOCH + 1), fact('finished', 'wx2', 3)]
        raw = ''.join(json.dumps(row) + '\n' for row in rows).encode()
        self.archive('wx2', 'source', {'./facts.jsonl': raw,
                     './status.json': status('wx2', {'peer:alpha': 2})})
        (self.root / 'wx2-facts.jsonl').write_bytes(raw[:70])
        summary = self.analyze()
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 2)
        self.assertTrue(summary['inputs']['wx2']['source']['complete'])
        self.assertIn('archive', summary['inputs']['wx2']['source']['origin'])
        self.assertTrue(any('mirror' in note for note in summary['notes']))
        self.assertFalse(any('mirror' in issue for issue in summary['issues']))

    def complete_session(self, resource_raw=None, hci_raw=BTSNOOP_HEADER):
        for alias, peer in [('wx1', 'wx2'), ('wx2', 'wx1')]:
            rows = [fact('started', alias, 0), fact('self', alias, .1, EPOCH),
                    fact('peer', alias, .2, EPOCH), fact('finished', alias, 1)]
            counts = {'self': 1, 'peer:' + TARGETS[alias]['peer_id']: 1}
            rows[-1]['counts'] = counts
            raw = ''.join(json.dumps(row) + '\n' for row in rows).encode()
            self.archive(alias, 'source', {'./facts.jsonl': raw, './status.json': status(alias, counts)})
            (self.root / f'{alias}-facts.jsonl').write_bytes(raw[:100])
            radio = {'epoch_ns': EPOCH + 500000000, 'elapsed_s': .5,
                     'expected_peer_address': TARGETS[peer]['mac'],
                     'hci_process_alive': True, 'resource_process_alive': True,
                     'connected': True, 'services_resolved': True, 'rssi_dbm_cached': -55}
            resources = {'epoch_ns': EPOCH + 500000000, 'processes': [
                {'name': 'peer-information', 'cpu_percent_one_core': 1, 'rss_bytes': 1048576}]}
            members = {'./radio.jsonl': (json.dumps(radio) + '\n').encode(),
                       './resources.jsonl': (json.dumps(resources) + '\n').encode() if resource_raw is None else resource_raw,
                       './finished.json': {'finished': True, 'epoch_ns': EPOCH + 1000000000}}
            if hci_raw is not None:
                members['./hci.btsnoop'] = hci_raw
            self.archive(alias, 'radio', members)
        vehicles = {'wx1': snapshot('wx1', 0, seconds=.5), 'wx2': snapshot('wx2', 2500, seconds=.5)}
        for alias, vehicle in vehicles.items():
            vehicle['counts'] = {'self': 1, 'peer:' + TARGETS[alias]['peer_id']: 1}
        self.write_rows('feedback.jsonl', [{'epoch_ns': EPOCH + 500000000, 'vehicles': vehicles}])

    def test_complete_canonical_recording_is_not_downgraded_by_expected_mirror_lag(self):
        self.complete_session()
        summary = self.analyze()
        self.assertEqual(summary['data_status'], 'complete')
        self.assertEqual(summary['issues'], [])
        self.assertEqual(len(summary['notes']), 2)
        self.assertTrue(all(evidence[kind]['complete'] for evidence in summary['inputs'].values()
                            for kind in ['source', 'radio']))
        self.assertEqual(summary['directions']['wx1->wx2']['matched_receiver_events'], 1)
        self.assertEqual(summary['distance']['valid_snapshots'], 1)

    def test_finished_invalid_and_api_error_events_are_quality_gaps_with_valid_measurements_retained(self):
        self.write_rows('wx1-facts.jsonl', [fact('self', 'wx1', 0, EPOCH), fact('self', 'wx1', .1, EPOCH + 1)])
        rows = [fact('started', 'wx2', 0), fact('peer', 'wx2', 1, EPOCH),
                {**fact('invalid', 'wx2', 2), 'raw': 'invalid-fact', 'error': 'invalid footprint'},
                {**fact('api_error', 'wx2', 3), 'error': 'status query timed out'},
                {**fact('api_error', 'wx2', 4), 'error': 'status query timed out'},
                fact('peer', 'wx2', 5, EPOCH + 1), fact('finished', 'wx2', 6)]
        counts = {'self': 0, 'peer:alpha': 2, 'invalid': 1}
        rows[-1]['counts'] = counts
        self.archive('wx2', 'source', {'./facts.jsonl': ''.join(json.dumps(row) + '\n' for row in rows).encode(),
                     './status.json': status('wx2', counts)})
        summary = self.analyze()
        source = summary['inputs']['wx2']['source']
        self.assertFalse(source['complete'])
        self.assertTrue(source['capture_complete'])
        self.assertEqual(source['data_quality']['status'], 'degraded')
        self.assertEqual(source['data_quality']['sampling_error_counts'], {'invalid': 1, 'api_error': 2})
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 2)
        self.assertEqual(summary['directions']['wx1->wx2']['matched_receiver_events'], 2)
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_interarrival_s']['max'], 4)
        self.assertTrue(any('wx2/source' in issue and 'invalid' in issue and '1' in issue for issue in summary['issues']))
        self.assertTrue(any('wx2/source' in issue and 'api_error' in issue and '2' in issue for issue in summary['issues']))

    def test_invalid_only_capture_is_complete_but_quality_is_degraded(self):
        rows = [fact('started', 'wx2', 0), {**fact('invalid', 'wx2', 1), 'raw': '{}', 'error': 'invalid geometry'},
                fact('finished', 'wx2', 2)]
        counts = {'self': 0, 'peer:alpha': 0, 'invalid': 1}
        rows[-1]['counts'] = counts
        self.archive('wx2', 'source', {'./facts.jsonl': ''.join(json.dumps(row) + '\n' for row in rows).encode(),
                     './status.json': status('wx2', counts)})
        source = self.analyze()['inputs']['wx2']['source']
        self.assertFalse(source['complete'])
        self.assertTrue(source['capture_complete'])
        self.assertEqual(source['data_quality']['sampling_error_counts']['invalid'], 1)

    def test_terminal_invalid_counters_are_checked_in_both_records_and_allow_explicit_zero(self):
        for status_count, finish_count, complete in [(0, 0, True), (1, 0, False), (0, 1, False)]:
            with self.subTest(status_count=status_count, finish_count=finish_count):
                rows = [fact('started', 'wx2', 0), fact('finished', 'wx2', 1)]
                rows[-1]['counts'] = {'self': 0, 'peer:alpha': 0, 'invalid': finish_count}
                self.archive('wx2', 'source', {'./facts.jsonl': ''.join(json.dumps(row) + '\n' for row in rows).encode(),
                             './status.json': status('wx2', {'self': 0, 'peer:alpha': 0, 'invalid': status_count})})
                summary = self.analyze()
                self.assertEqual(summary['inputs']['wx2']['source']['capture_complete'], complete)
                self.assertEqual(summary['inputs']['wx2']['source']['complete'], complete)
                if not complete:
                    self.assertTrue(any('count mismatch for invalid' in issue for issue in summary['issues']))

    def test_captured_malformed_geometry_does_not_become_a_missing_log_count(self):
        row = fact('peer', 'wx2', 1, EPOCH)
        row['data']['geometry'].pop('velocity')
        rows = [fact('started', 'wx2', 0), row, fact('finished', 'wx2', 2)]
        counts = {'self': 0, 'peer:alpha': 1, 'invalid': 0}
        rows[-1]['counts'] = counts
        self.archive('wx2', 'source', {'./facts.jsonl': ''.join(json.dumps(row) + '\n' for row in rows).encode(),
                     './status.json': status('wx2', counts)})
        summary = self.analyze()
        source = summary['inputs']['wx2']['source']
        self.assertTrue(source['capture_complete'])
        self.assertFalse(source['complete'])
        self.assertEqual(source['data_quality']['status'], 'degraded')
        self.assertFalse(any('count mismatch' in issue for issue in summary['issues']))
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 0)

    def test_empty_resources_cannot_prove_coverage_despite_alive_and_finished(self):
        self.complete_session(resource_raw=b'')
        summary = self.analyze()
        self.assertEqual(summary['data_status'], 'incomplete')
        self.assertEqual(summary['vehicles']['wx1']['resources']['samples'], 0)
        self.assertEqual(summary['inputs']['wx1']['radio']['capture_coverage']['resources']['status'], 'unavailable')
        self.assertTrue(any('resources' in issue and 'empty' in issue for issue in summary['issues']))
        self.assertEqual(summary['directions']['wx1->wx2']['matched_receiver_events'], 1)

    def test_hci_basic_header_and_size_allow_valid_zero_packets_and_reject_missing_or_truncated(self):
        for raw, valid in [(BTSNOOP_HEADER, True), (None, False), (b'btsnoop\0', False),
                           (b'invalid-header!!', False), (BTSNOOP_HEADER + b'x', False),
                           (b'btsnoop\0' + (2).to_bytes(4, 'big') + (2001).to_bytes(4, 'big'), False)]:
            with self.subTest(raw=raw):
                self.complete_session(hci_raw=raw)
                summary = self.analyze()
                self.assertEqual(summary['inputs']['wx1']['radio']['complete'], valid)
                self.assertEqual(summary['data_status'], 'complete' if valid else 'incomplete')
                if valid:
                    self.assertEqual(summary['inputs']['wx1']['radio']['hci_capture']['size_bytes'], 16)
                    self.assertEqual(summary['inputs']['wx1']['radio']['hci_capture']['validation'], 'basic_header_and_size_only')
                else:
                    self.assertTrue(any('hci.btsnoop' in issue for issue in summary['issues']))

    def test_invalid_partial_lines_do_not_discard_valid_prefix(self):
        self.write_rows('wx2-facts.jsonl', [fact('peer', 'wx2', 1, EPOCH)])
        with (self.root / 'wx2-facts.jsonl').open('a') as raw:
            raw.write('{"kind":')
        summary = self.analyze()
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 1)
        self.assertFalse(summary['inputs']['wx2']['source']['complete'])
        self.assertTrue(any('invalid JSON' in issue for issue in summary['issues']))

    def test_wrong_fact_and_archive_identity_are_rejected(self):
        wrong = fact('peer', 'wx2', 1, EPOCH)
        wrong['own_id'] = 'another-robot'
        self.write_rows('wx2-facts.jsonl', [wrong, fact('peer', 'wx2', 2, EPOCH, 'stranger')])
        self.assertEqual(self.analyze()['directions']['wx1->wx2']['receiver_peer_events'], 0)
        self.archive('wx2', 'source', {'./facts.jsonl': (json.dumps(fact('peer', 'wx2', 2, EPOCH)) + '\n').encode(),
                     './status.json': status('wx1')})
        summary = self.analyze()
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 0)
        self.assertTrue(any('identity' in issue for issue in summary['issues']))

    def test_third_peer_counts_are_retained_without_affecting_selected_pair(self):
        self.write_rows('wx1-facts.jsonl', [fact('self', 'wx1', 0, EPOCH), fact('self', 'wx1', 1, EPOCH + 1)])
        rows = [fact('started', 'wx2', 0), fact('peer', 'wx2', 1, EPOCH),
                fact('peer', 'wx2', 2, EPOCH, 'charlie'), fact('peer', 'wx2', 3, EPOCH, 'charlie'),
                fact('peer', 'wx2', 4, EPOCH + 1), fact('finished', 'wx2', 5)]
        counts = {'peer:alpha': 2, 'peer:charlie': 2}
        rows[-1]['counts'] = counts
        self.archive('wx2', 'source', {'./facts.jsonl': ''.join(json.dumps(row) + '\n' for row in rows).encode(),
                     './status.json': status('wx2', counts)})
        summary = self.analyze()
        source = summary['inputs']['wx2']['source']
        self.assertTrue(source['complete'])
        self.assertEqual(source['peer_events_by_robot'], {'alpha': 2, 'charlie': 2})
        self.assertEqual(source['other_peer_events_by_robot'], {'charlie': 2})
        direction = summary['directions']['wx1->wx2']
        self.assertEqual(direction['receiver_peer_events'], 2)
        self.assertEqual(direction['matched_receiver_events'], 2)
        self.assertEqual(direction['receiver_interarrival_s']['max'], 3)
        self.assertFalse(any('wx2/source' in issue for issue in summary['issues']))

    def test_third_peer_terminal_counter_mismatch_is_reported_by_actual_identity(self):
        rows = [fact('peer', 'wx2', 1, EPOCH, 'charlie'), fact('finished', 'wx2', 2)]
        self.archive('wx2', 'source', {'./facts.jsonl': ''.join(json.dumps(row) + '\n' for row in rows).encode(),
                     './status.json': status('wx2', {'peer:charlie': 2})})
        summary = self.analyze()
        self.assertTrue(any('count mismatch for peer:charlie' in issue for issue in summary['issues']))
        self.assertFalse(summary['inputs']['wx2']['source']['complete'])

    def test_peer_outer_and_embedded_identity_mismatch_is_invalid(self):
        row = fact('peer', 'wx2', 1, EPOCH, 'charlie')
        row['data']['robot_id'] = 'delta'
        self.write_rows('wx2-facts.jsonl', [row])
        summary = self.analyze()
        self.assertTrue(any('fact robot identity mismatch' in issue for issue in summary['issues']))
        self.assertEqual(summary['inputs']['wx2']['source']['rejected_rows'], 1)

    def test_incomplete_or_malformed_local_geometry_is_rejected_explicitly(self):
        cases = [('map_id', None), ('map_id', ''), ('pose', None), ('pose', [0, 0]),
                 ('pose', [0, float('nan'), 0]), ('velocity', None), ('velocity', [0, True, 0]),
                 ('footprint', []), ('footprint', [[0], [1, 0], [0, 1]]),
                 ('sampled_at_ns', 2 ** 63)]
        for kind in ['self', 'peer']:
            for field, value in cases:
                with self.subTest(kind=kind, field=field, value=value):
                    row = fact(kind, 'wx2', 1, EPOCH)
                    geometry = row['data'] if kind == 'self' else row['data']['geometry']
                    if value is None:
                        geometry.pop(field)
                    else:
                        geometry[field] = value
                    self.write_rows('wx2-facts.jsonl', [row, fact('finished', 'wx2', 2)])
                    summary = self.analyze()
                    source = summary['inputs']['wx2']['source']
                    self.assertFalse(source['complete'])
                    self.assertEqual(source['rejected_rows'], 1)
                    self.assertTrue(any('geometry' in issue.lower() for issue in summary['issues']))

    def test_resource_units_and_rssi_are_explicit(self):
        processes = [{'name': 'peer-information', 'pid': 1, 'cpu_percent_one_core': None, 'rss_bytes': 1048576},
                     {'name': 'easygo-app', 'pid': 2, 'cpu_percent_one_core': 125, 'rss_bytes': 2097152}]
        self.archive('wx1', 'radio', {'./resources.jsonl': (json.dumps({'processes': processes}) + '\n').encode(),
                     './radio.jsonl': (json.dumps({'expected_peer_address': TARGETS['wx2']['mac'],
                                                  'rssi_dbm_cached': -72}) + '\n').encode()})
        vehicle = self.analyze()['vehicles']['wx1']
        self.assertEqual(vehicle['resources']['by_process']['easygo-app']['cpu_percent_one_core']['max'], 125)
        self.assertEqual(vehicle['resources']['by_process']['peer-information']['rss_mib']['mean'], 1)
        self.assertIsNone(vehicle['resources']['by_process']['peer-information']['cpu_percent_one_core']['mean'])
        self.assertEqual(vehicle['radio']['rssi_dbm_cached']['min'], -72)
        self.assertIn('cached', vehicle['radio']['rssi_basis'])

    def test_resource_totals_keep_unknown_cpu_distinct_from_zero(self):
        rows = [{'processes': [{'name': 'peer-information', 'cpu_percent_one_core': None, 'rss_bytes': 1048576}]},
                {'processes': [{'name': 'peer-information', 'cpu_percent_one_core': 125, 'rss_bytes': 1048576},
                               {'name': 'easygo-app', 'cpu_percent_one_core': 25, 'rss_bytes': 3145728}]}]
        self.write_rows('wx1-resources.jsonl', rows)
        resources = self.analyze()['vehicles']['wx1']['resources']
        self.assertEqual(resources['tracked_process_totals']['cpu_percent_one_core']['count'], 1)
        self.assertEqual(resources['tracked_process_totals']['cpu_percent_one_core']['max'], 150)
        self.assertEqual(resources['tracked_process_totals']['rss_mib']['max'], 4)

    def test_finished_radio_loop_preserves_measurements_but_marks_child_failures_partial(self):
        healthy = {'epoch_ns': EPOCH, 'elapsed_s': 0,
                   'expected_peer_address': TARGETS['wx2']['mac'],
                   'hci_process_alive': True, 'resource_process_alive': True,
                   'connected': True, 'services_resolved': True, 'rssi_dbm_cached': -55}
        cases = [({}, True, None), ({'hci_process_alive': False}, False, 'hci_process_alive'),
                 ({'resource_process_alive': False}, False, 'resource_process_alive'),
                 ({'error': 'properties query failed', 'rssi_dbm_cached': None}, False, 'radio_properties')]
        for changes, complete, affected in cases:
            with self.subTest(changes=changes):
                second = {**healthy, 'epoch_ns': EPOCH + 1000000000, 'elapsed_s': 1, **changes}
                resources = {'processes': [{'name': 'peer-information', 'cpu_percent_one_core': 25,
                                            'rss_bytes': 2097152}]}
                self.archive('wx1', 'radio', {
                    './radio.jsonl': ''.join(json.dumps(row) + '\n' for row in [healthy, second]).encode(),
                    './resources.jsonl': (json.dumps(resources) + '\n').encode(),
                    './hci.btsnoop': BTSNOOP_HEADER,
                    './finished.json': {'finished': True, 'epoch_ns': EPOCH + 2000000000},
                })
                summary = self.analyze()
                metadata = summary['inputs']['wx1']['radio']
                self.assertEqual(metadata['complete'], complete)
                if affected:
                    self.assertEqual(metadata['capture_coverage'][affected]['status'], 'partial')
                    self.assertTrue(any(affected in issue for issue in summary['issues']))
                vehicle = summary['vehicles']['wx1']
                self.assertEqual(vehicle['radio']['samples'], 2)
                self.assertEqual(vehicle['radio']['rssi_dbm_cached']['mean'], -55)
                self.assertEqual(vehicle['resources']['tracked_process_totals']['cpu_percent_one_core']['mean'], 25)
                self.assertEqual(vehicle['resources']['tracked_process_totals']['rss_mib']['mean'], 2)

    def test_missing_radio_child_health_is_unknown_coverage(self):
        row = {'expected_peer_address': TARGETS['wx2']['mac'], 'rssi_dbm_cached': -55}
        self.archive('wx1', 'radio', {'./radio.jsonl': (json.dumps(row) + '\n').encode(),
                     './resources.jsonl': b'', './finished.json': {'finished': True}})
        metadata = self.analyze()['inputs']['wx1']['radio']
        self.assertFalse(metadata['complete'])
        self.assertEqual(metadata['capture_coverage']['hci_process_alive']['status'], 'unknown')

    def test_malformed_resource_and_radio_rows_are_reported_without_crashing(self):
        self.write_rows('wx1-resources.jsonl', [{'processes': None}, {'processes': [None]},
                        {'processes': [{'name': 'peer-information', 'cpu_percent_one_core': 10 ** 400,
                                        'rss_bytes': -100}]}])
        self.write_rows('wx1-radio.jsonl', [{'expected_peer_address': None, 'rssi_dbm_cached': -50}])
        summary = self.analyze()
        self.assertEqual(summary['vehicles']['wx1']['radio']['samples'], 0)
        self.assertTrue(any('invalid process' in issue for issue in summary['issues']))
        self.assertIsNone(summary['vehicles']['wx1']['resources']['by_process']['peer-information']['rss_mib']['mean'])

    def test_paired_distance_is_recomputed_and_low_or_unknown_confidence_is_flagged(self):
        rows = [{'epoch_ns': EPOCH, 'vehicles': {'wx1': snapshot('wx1', 0),
                                                'wx2': snapshot('wx2', 2500, 20)},
                 'summary': {'distance_m': 999}},
                {'epoch_ns': EPOCH + 5000000000, 'vehicles': {'wx1': snapshot('wx1', 0),
                                                            'wx2': snapshot('wx2', 3500, None)}}]
        for value in rows[1]['vehicles'].values():
            value['query_start_ns'] += 5000000000
            value['query_end_ns'] += 5000000000
        self.write_rows('feedback.jsonl', rows)
        summary = self.analyze()
        self.assertEqual(summary['distance']['valid_snapshots'], 2)
        with (self.root / 'analysis' / 'timeline.csv').open() as data:
            timeline = list(csv.DictReader(data))
        self.assertEqual(float(timeline[0]['distance_m']), 2.5)
        self.assertIn('low_confidence', timeline[0]['flags'])
        self.assertIn('unknown_confidence', timeline[1]['flags'])
        with (self.root / 'analysis' / 'distance-bins.csv').open() as data:
            bins = list(csv.DictReader(data))
        self.assertEqual([float(row['lower_m']) for row in bins], [2, 3])
        self.assertEqual(summary['distance']['estimated_snapshots'], 2)

    def test_timeline_counter_deltas_and_bins_preserve_zero_unknown_and_reset(self):
        rows = []
        for index, (forward, reverse) in enumerate(zip([5, 5, 8, None, 10, 2, 3], [0, 0, 0, 0, None, 0, 0])):
            vehicles = {'wx1': snapshot('wx1', 0, seconds=index * 5),
                        'wx2': snapshot('wx2', 2500, seconds=index * 5)}
            for alias, count in [('wx2', forward), ('wx1', reverse)]:
                vehicles[alias]['counts'] = {} if count is None else {'peer:' + TARGETS[alias]['peer_id']: count}
                vehicles[alias]['last_peer_gap_s'] = None if count is None else .25
            rows.append({'epoch_ns': EPOCH + index * 5000000000, 'vehicles': vehicles})
        self.write_rows('feedback.jsonl', rows)
        summary = self.analyze()
        with (self.root / 'analysis' / 'timeline.csv').open() as data:
            timeline = list(csv.DictReader(data))
        self.assertEqual([row['wx1_to_wx2_peer_count'] for row in timeline], ['5', '5', '8', '', '10', '2', '3'])
        self.assertEqual([row['wx1_to_wx2_peer_delta'] for row in timeline], ['', '0', '3', '', '2', '', '1'])
        self.assertEqual(timeline[0]['wx2_to_wx1_peer_count'], '0')
        self.assertEqual(timeline[4]['wx2_to_wx1_peer_count'], '')
        self.assertEqual(timeline[0]['wx1_to_wx2_observed_gap_s'], '0.25')
        self.assertEqual(timeline[3]['wx1_to_wx2_observed_gap_s'], '')
        self.assertIn('counter_reset', timeline[5]['flags'])
        with (self.root / 'analysis' / 'distance-bins.csv').open() as data:
            bucket = list(csv.DictReader(data))[0]
        self.assertEqual(bucket['snapshots'], '7')
        self.assertEqual(bucket['wx1_to_wx2_polls_with_new_frames'], '3')
        self.assertEqual(bucket['wx1_to_wx2_polls_without_new_frames'], '1')
        self.assertEqual(bucket['wx1_to_wx2_polls_unknown_delta'], '3')
        self.assertIn('previous valid counter snapshot', summary['distance']['bin_basis'])
        self.assertIn('not packet delivery probability', summary['distance']['bin_basis'])
        self.assertIn('snapshot distance', (self.root / 'analysis' / 'report.md').read_text())

    def test_wrong_identity_feedback_does_not_contribute_counts_or_gap(self):
        rows = []
        for index, count in enumerate([5, 999, 6]):
            vehicles = {'wx1': snapshot('wx1', 0, seconds=index * 5),
                        'wx2': snapshot('wx2', 2500, seconds=index * 5)}
            vehicles['wx2']['counts'] = {'peer:alpha': count}
            if index == 1:
                vehicles['wx2']['own_id'] = 'stranger'
            rows.append({'epoch_ns': EPOCH + index * 5000000000, 'vehicles': vehicles})
        self.write_rows('feedback.jsonl', rows)
        self.analyze()
        with (self.root / 'analysis' / 'timeline.csv').open() as data:
            timeline = list(csv.DictReader(data))
        self.assertEqual(timeline[1]['wx1_to_wx2_peer_count'], '')
        self.assertEqual(timeline[1]['wx1_to_wx2_peer_delta'], '')
        self.assertEqual(timeline[1]['wx1_to_wx2_observed_gap_s'], '')
        self.assertEqual(timeline[2]['wx1_to_wx2_peer_delta'], '1')

    def test_as_started_transport_and_versions_are_preserved_with_unknown_defaults(self):
        targets = {key: dict(value) for key, value in TARGETS.items()}
        targets['wx1'].update(transport={'mode': 'ble', 'selection': 'environment'},
                              package_versions={'peer-information': '0.1.2', 'easygo-app': '1.2.3'})
        self.write_json('session.json', {'targets': targets})
        started = self.analyze()['as_started_targets']
        self.assertEqual(started['wx1']['transport'], {'mode': 'ble', 'selection': 'environment'})
        self.assertEqual(started['wx1']['package_versions'], {'peer-information': '0.1.2', 'easygo-app': '1.2.3'})
        self.assertIsNone(started['wx2']['transport'])
        self.assertIsNone(started['wx2']['package_versions'])

    def test_wrong_map_stale_missing_and_wrong_snapshot_identity_have_no_distance(self):
        good = snapshot('wx1', 0)
        other = snapshot('wx2', 3000, map_id='other-map')
        stale = snapshot('wx2', 3000)
        stale['vehicle']['epoch_ns'] -= 3000000000
        wrong = snapshot('wx2', 3000)
        wrong['own_id'] = 'stranger'
        self.write_rows('feedback.jsonl', [{'epoch_ns': EPOCH, 'vehicles': vehicles} for vehicles in
                        [{'wx1': good, 'wx2': other}, {'wx1': good, 'wx2': stale}, {'wx1': good},
                         {'wx1': good, 'wx2': wrong}]])
        summary = self.analyze()
        self.assertEqual(summary['distance']['valid_snapshots'], 0)
        with (self.root / 'analysis' / 'timeline.csv').open() as data:
            flags = [row['flags'] for row in csv.DictReader(data)]
        self.assertIn('map_mismatch', flags[0])
        self.assertIn('stale', flags[1])
        self.assertIn('missing', flags[2])
        self.assertIn('identity', flags[3])

    def test_missing_poll_freshness_and_stale_poll_cannot_create_distance(self):
        missing = snapshot('wx2', 3000)
        missing.pop('remote_status_age_s')
        stale = snapshot('wx2', 3000)
        stale['query_start_ns'] -= 10000000000
        stale['query_end_ns'] -= 10000000000
        self.write_rows('feedback.jsonl', [{'epoch_ns': EPOCH, 'vehicles': {
            'wx1': snapshot('wx1', 0), 'wx2': other}} for other in [missing, stale]])
        self.assertEqual(self.analyze()['distance']['valid_snapshots'], 0)

    def test_corrupt_canonical_archive_never_silently_falls_back_to_mirror(self):
        (self.root / 'wx2-source.tar.gz').write_bytes(b'incomplete')
        self.write_rows('wx2-facts.jsonl', [fact('peer', 'wx2', 1, EPOCH)])
        summary = self.analyze()
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 0)
        self.assertFalse(summary['inputs']['wx2']['source']['complete'])
        self.assertTrue(any('invalid/incomplete archive' in issue for issue in summary['issues']))

    def test_finished_status_count_mismatch_marks_recording_incomplete(self):
        rows = [fact('started', 'wx2', 0), fact('peer', 'wx2', 1, EPOCH), fact('finished', 'wx2', 2)]
        self.archive('wx2', 'source', {'./facts.jsonl': ''.join(json.dumps(row) + '\n' for row in rows).encode(),
                     './status.json': status('wx2', {'peer:alpha': 5})})
        summary = self.analyze()
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 1)
        self.assertFalse(summary['inputs']['wx2']['source']['complete'])
        self.assertTrue(any('count mismatch' in issue for issue in summary['issues']))

    def test_safe_tar_members_are_read_without_extracting_links_or_traversal(self):
        rows = (json.dumps(fact('peer', 'wx2', 1, EPOCH)) + '\n').encode()
        self.archive('wx2', 'source', {'../escaped-analysis-test': b'bad', './facts.jsonl': rows})
        with tarfile.open(self.root / 'wx1-source.tar.gz', 'w:gz') as archive:
            member = tarfile.TarInfo('./facts.jsonl')
            member.type = tarfile.SYMTYPE
            member.linkname = '/etc/passwd'
            archive.addfile(member)
        summary = self.analyze()
        self.assertEqual(summary['directions']['wx1->wx2']['receiver_peer_events'], 1)
        self.assertEqual(summary['directions']['wx2->wx1']['receiver_peer_events'], 0)
        self.assertFalse((self.root.parent / 'escaped-analysis-test').exists())
        self.assertTrue(any('unsafe' in issue for issue in summary['issues']))

    def test_invalid_session_does_not_assign_data_to_unknown_vehicles(self):
        (self.root / 'session.json').write_text('{')
        self.write_rows('wx2-facts.jsonl', [fact('peer', 'wx2', 1, EPOCH)])
        summary = self.analyze()
        self.assertEqual(summary['directions'], {})
        self.assertTrue(any('session.json' in issue for issue in summary['issues']))


if __name__ == '__main__':
    unittest.main()
