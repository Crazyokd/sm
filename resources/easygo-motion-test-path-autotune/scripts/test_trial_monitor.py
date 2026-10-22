import math
import unittest
from unittest.mock import Mock, patch
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from trial_monitor import (
    Thresholds,
    TrialAnalyzer,
    build_api_source,
    check_preflight,
    has_preactivation_terminal,
    main,
    readline_until,
    task_is_active,
)


def sample(x, y, yaw, vx=0.0, omega=0.0, *, action="RUNNING", estop="NONE", field=False):
    return {
        "agvPosition": {
            "positionInitialized": True,
            "localizationScore": 0.8,
            "x": x,
            "y": y,
            "theta": yaw,
        },
        "velocity": {"vx": vx, "vy": 0.0, "omega": omega},
        "actionStates": [{"actionType": "motionTestPath", "actionStatus": action}],
        "errors": [],
        "safetyState": {"eStop": estop, "fieldViolation": field},
    }


def smooth_loop():
    return [
        sample(0.0, 0.0, 0.0),
        sample(0.2, 0.0, 0.01, 0.2, 0.02),
        sample(1.5, 0.0, 0.01, 0.8, 0.01),
        sample(2.9, 0.0, 0.015, 0.3, -0.01),
        sample(3.01, 0.0, 0.02),
        sample(3.01, 0.0, 1.2, 0.0, 0.8),
        sample(3.01, 0.0, math.pi, 0.0, 0.0),
        sample(2.8, 0.0, math.pi - 0.01, 0.2, 0.01),
        sample(1.4, 0.0, math.pi - 0.01, 0.8, -0.01),
        sample(0.1, 0.0, math.pi - 0.02, 0.3, 0.01),
        sample(-0.005, 0.0, math.pi - 0.02),
        sample(-0.005, 0.0, -1.4, 0.0, 0.8),
        sample(-0.005, 0.0, 0.01),
    ]


class TrialAnalyzerTest(unittest.TestCase):
    def test_readline_timeout_does_not_block_on_silent_stream(self):
        stream = Mock()
        with patch("trial_monitor.select.select", return_value=([], [], [])):
            self.assertIsNone(readline_until(stream, 1.0))

        stream.readline.assert_not_called()

    def test_remote_state_stream_has_a_hard_timeout(self):
        with patch("trial_monitor.subprocess.Popen") as popen:
            from trial_monitor import RemoteRobot

            RemoteRobot("sz-2").states(12)

        self.assertIn("timeout --signal=TERM 12s", popen.call_args.args[0][-1])

    def test_monitor_failure_stops_vehicle(self):
        class Robot:
            def __init__(self, _host):
                self.stopped = False
                self.confirmed_stopped = False

            def status(self):
                return None

            def sample_state(self):
                return sample(0.0, 0.0, 0.0)

            def start(self, _task_id):
                return {}

            def stop(self):
                self.stopped = True

            def wait_stopped(self):
                self.confirmed_stopped = True

        robot = Robot("sz-2")
        argv = ["trial_monitor.py", "trial", "--task-id", "task-id"]
        with (
            patch("trial_monitor.RemoteRobot", return_value=robot),
            patch("trial_monitor.monitor_trial", side_effect=RuntimeError("timeout")),
            patch.object(sys, "argv", argv),
            self.assertRaisesRegex(RuntimeError, "timeout"),
        ):
            main()

        self.assertTrue(robot.stopped)
        self.assertTrue(robot.confirmed_stopped)

    def test_cancelled_history_does_not_block_next_trial(self):
        self.assertFalse(task_is_active({"phase": "cancelled"}))
        self.assertTrue(task_is_active({"phase": "running"}))

    def test_preflight_ignores_terminal_action_history(self):
        check_preflight(sample(0.0, 0.0, 0.0, action="FAILED"), Thresholds())

    def test_preflight_ignores_terminal_error_history(self):
        state = sample(0.0, 0.0, 0.0, action="FAILED")
        state["errors"] = [{"errorLevel": "FATAL"}]

        check_preflight(state, Thresholds())

    def test_runtime_error_terminates_while_action_is_waiting(self):
        state = sample(0.0, 0.0, 0.0, action="WAITING")
        state["errors"] = [{"errorLevel": "FATAL"}]

        self.assertTrue(has_preactivation_terminal(state, Thresholds()))

    def test_get_api_source_uses_python_none(self):
        source = build_api_source(18089, "GET", "/api/v6/tasks/current/status", None)

        self.assertIn("data = None", source)
        self.assertNotIn("data = null", source)

    def test_accepts_one_smooth_closed_loop(self):
        analyzer = TrialAnalyzer(loops_required=1, thresholds=Thresholds())
        for item in smooth_loop():
            analyzer.feed(item)

        report = analyzer.report()

        self.assertTrue(report.accepted)
        self.assertEqual(report.loops, 1)
        self.assertEqual(len(report.line_endpoints), 2)
        self.assertLess(report.p95_position_error_m, 0.03)
        self.assertLess(report.p95_heading_error_deg, 5.0)
        self.assertGreaterEqual(report.median_peak_speed_mps, 0.76)

    def test_rotation_linear_noise_does_not_create_line_segments(self):
        records = smooth_loop()
        records[5] = sample(3.01, 0.0, 1.2, 0.09, 0.8)
        records[11] = sample(-0.005, 0.0, -1.4, 0.09, 0.8)
        analyzer = TrialAnalyzer(loops_required=1, thresholds=Thresholds())
        for item in records:
            analyzer.feed(item)

        report = analyzer.report()

        self.assertEqual(report.loops, 1)
        self.assertEqual(len(report.line_endpoints), 2)
        self.assertTrue(report.accepted)

    def test_terminal_angular_correction_is_not_the_next_rotation(self):
        records = smooth_loop()
        records.insert(4, sample(2.94, 0.0, -0.05, 0.10, 0.30))
        analyzer = TrialAnalyzer(loops_required=1, thresholds=Thresholds())
        for item in records:
            analyzer.feed(item)

        report = analyzer.report()

        self.assertTrue(report.accepted)
        self.assertEqual(report.loops, 1)
        self.assertLess(report.line_endpoints[0].position_error_m, 0.03)

    def test_rejects_large_terminal_heading_error(self):
        records = smooth_loop()
        records[4] = sample(3.01, 0.0, 0.2)
        analyzer = TrialAnalyzer(loops_required=1, thresholds=Thresholds())
        for item in records:
            analyzer.feed(item)

        report = analyzer.report()

        self.assertFalse(report.accepted)
        self.assertIn("heading_error", report.reasons)

    def test_safety_violation_requests_immediate_stop(self):
        analyzer = TrialAnalyzer(loops_required=1, thresholds=Thresholds())
        analyzer.feed(sample(0.0, 0.0, 0.0))
        analyzer.feed(sample(0.1, 0.0, 0.0, 0.1, field=True))

        report = analyzer.report()

        self.assertFalse(report.accepted)
        self.assertTrue(report.stop_required)
        self.assertEqual(report.terminal_reason, "safety_violation")

    def test_external_action_stop_terminates_without_restart(self):
        analyzer = TrialAnalyzer(loops_required=1, thresholds=Thresholds())
        analyzer.feed(sample(0.0, 0.0, 0.0))
        analyzer.feed(sample(0.0, 0.0, 0.0, action="FAILED"))

        report = analyzer.report()

        self.assertFalse(report.accepted)
        self.assertFalse(report.stop_required)
        self.assertEqual(report.terminal_reason, "action_failed")


if __name__ == "__main__":
    unittest.main()
