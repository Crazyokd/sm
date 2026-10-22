#!/usr/bin/env python3
import argparse
import base64
import json
import math
import re
import select
import statistics
import subprocess
import sys
import time
from dataclasses import asdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


def normalize_angle(value: float) -> float:
    return math.atan2(math.sin(value), math.cos(value))


def readline_until(stream: Any, deadline: float) -> str | None:
    ready, _, _ = select.select([stream], [], [], max(0.0, deadline - time.monotonic()))
    return stream.readline() if ready else None


def percentile(values: list[float], quantile: float) -> float:
    if not values:
        return math.inf
    ordered = sorted(values)
    index = max(0, math.ceil(quantile * len(ordered)) - 1)
    return ordered[index]


@dataclass(frozen=True)
class Thresholds:
    max_p95_position_error_m: float = 0.03
    max_p95_heading_error_deg: float = 5.0
    max_pullback_m: float = 0.02
    min_median_peak_speed_mps: float = 0.76
    min_localization_score: float = 0.5


@dataclass(frozen=True)
class LineEndpoint:
    index: int
    position_error_m: float
    heading_error_deg: float
    pullback_m: float
    peak_speed_mps: float


@dataclass(frozen=True)
class TrialReport:
    accepted: bool
    loops: int
    line_endpoints: list[LineEndpoint]
    p95_position_error_m: float
    p95_heading_error_deg: float
    max_pullback_m: float
    median_peak_speed_mps: float
    reasons: list[str] = field(default_factory=list)
    terminal_reason: str | None = None
    stop_required: bool = False


class TrialAnalyzer:
    def __init__(
        self,
        loops_required: int,
        thresholds: Thresholds,
        line_length_m: float = 3.0,
    ) -> None:
        self.loops_required = loops_required
        self.thresholds = thresholds
        self.line_length_m = line_length_m
        self.anchor: tuple[float, float, float] | None = None
        self.active_motion: str | None = None
        self.motion_samples: list[dict[str, Any]] = []
        self.line_endpoints: list[LineEndpoint] = []
        self.completed_rotations = 0
        self.terminal_reason: str | None = None
        self.stop_required = False

    def feed(self, state: dict[str, Any]) -> None:
        if self.terminal_reason:
            return
        self._check_runtime(state)
        if self.terminal_reason:
            return

        pose = state["agvPosition"]
        if self.anchor is None:
            self.anchor = (float(pose["x"]), float(pose["y"]), float(pose["theta"]))

        motion = self._motion_kind(state)
        if self.active_motion == "line":
            if motion is None:
                motion = "line"
            elif motion == "rotate":
                local_x, local_y = self._to_local(float(pose["x"]), float(pose["y"]))
                expected_x = self.line_length_m if len(self.line_endpoints) % 2 == 0 else 0.0
                if math.hypot(local_x - expected_x, local_y) > self.thresholds.max_p95_position_error_m:
                    motion = "line"
        elif self.active_motion == "rotate" and motion is None:
            motion = None if self._rotation_finished(pose) else "rotate"
        if motion == self.active_motion:
            if motion:
                self.motion_samples.append(state)
            return

        if self.active_motion == "line":
            endpoint_state = state if motion is None else self.motion_samples[-1]
            self._finish_line(endpoint_state)
        elif self.active_motion == "rotate":
            self.completed_rotations += 1

        self.active_motion = motion
        self.motion_samples = [state] if motion else []

    def report(self) -> TrialReport:
        positions = [item.position_error_m for item in self.line_endpoints]
        headings = [item.heading_error_deg for item in self.line_endpoints]
        pullbacks = [item.pullback_m for item in self.line_endpoints]
        peaks = [item.peak_speed_mps for item in self.line_endpoints]
        p95_position = percentile(positions, 0.95)
        p95_heading = percentile(headings, 0.95)
        max_pullback = max(pullbacks, default=math.inf)
        median_peak = statistics.median(peaks) if peaks else 0.0
        loops = self.completed_rotations // 2
        reasons: list[str] = []

        if self.terminal_reason:
            reasons.append(self.terminal_reason)
        if loops < self.loops_required:
            reasons.append("insufficient_loops")
        if len(self.line_endpoints) < self.loops_required * 2:
            reasons.append("insufficient_line_endpoints")
        if p95_position > self.thresholds.max_p95_position_error_m:
            reasons.append("position_error")
        if p95_heading > self.thresholds.max_p95_heading_error_deg:
            reasons.append("heading_error")
        if max_pullback > self.thresholds.max_pullback_m:
            reasons.append("pullback")
        if median_peak < self.thresholds.min_median_peak_speed_mps:
            reasons.append("peak_speed")

        return TrialReport(
            accepted=not reasons,
            loops=loops,
            line_endpoints=list(self.line_endpoints),
            p95_position_error_m=p95_position,
            p95_heading_error_deg=p95_heading,
            max_pullback_m=max_pullback,
            median_peak_speed_mps=median_peak,
            reasons=reasons,
            terminal_reason=self.terminal_reason,
            stop_required=self.stop_required,
        )

    def _check_runtime(self, state: dict[str, Any]) -> None:
        safety = state.get("safetyState", {})
        pose = state.get("agvPosition", {})
        if (
            safety.get("eStop", "NONE") != "NONE"
            or bool(safety.get("fieldViolation"))
            or not pose.get("positionInitialized", False)
            or float(pose.get("localizationScore", 0.0)) < self.thresholds.min_localization_score
        ):
            self.terminal_reason = "safety_violation"
            self.stop_required = True
            return
        if state.get("errors"):
            self.terminal_reason = "runtime_error"
            return

        statuses = [
            item.get("actionStatus")
            for item in state.get("actionStates", [])
            if item.get("actionType") == "motionTestPath"
        ]
        if "FAILED" in statuses:
            self.terminal_reason = "action_failed"
        elif any(status in {"FINISHED", "CANCELLED"} for status in statuses):
            self.terminal_reason = "external_stop"

    @staticmethod
    def _motion_kind(state: dict[str, Any]) -> str | None:
        velocity = state.get("velocity", {})
        vx = abs(float(velocity.get("vx", 0.0)))
        omega = abs(float(velocity.get("omega", 0.0)))
        if omega >= 0.25 and vx < 0.15:
            return "rotate"
        if vx >= 0.15:
            return "line"
        return None

    def _rotation_finished(self, pose: dict[str, Any]) -> bool:
        assert self.anchor is not None
        expected_yaw = self.anchor[2] + (math.pi if self.completed_rotations % 2 == 0 else 0.0)
        return abs(normalize_angle(float(pose["theta"]) - expected_yaw)) <= math.radians(10.0)

    def _finish_line(self, endpoint_state: dict[str, Any]) -> None:
        assert self.anchor is not None
        line_index = len(self.line_endpoints)
        outbound = line_index % 2 == 0
        endpoint_pose = endpoint_state["agvPosition"]
        local_x, local_y = self._to_local(float(endpoint_pose["x"]), float(endpoint_pose["y"]))
        expected_x = self.line_length_m if outbound else 0.0
        expected_yaw = self.anchor[2] if outbound else self.anchor[2] + math.pi
        progress_values = []
        peak_speed = 0.0
        for item in self.motion_samples:
            pose = item["agvPosition"]
            x, _ = self._to_local(float(pose["x"]), float(pose["y"]))
            progress_values.append(x if outbound else self.line_length_m - x)
            peak_speed = max(peak_speed, abs(float(item["velocity"].get("vx", 0.0))))
        final_progress = local_x if outbound else self.line_length_m - local_x
        pullback = max(0.0, max(progress_values, default=final_progress) - final_progress)
        self.line_endpoints.append(
            LineEndpoint(
                index=line_index,
                position_error_m=math.hypot(local_x - expected_x, local_y),
                heading_error_deg=math.degrees(
                    abs(normalize_angle(float(endpoint_pose["theta"]) - expected_yaw))
                ),
                pullback_m=pullback,
                peak_speed_mps=peak_speed,
            )
        )

    def _to_local(self, x: float, y: float) -> tuple[float, float]:
        assert self.anchor is not None
        dx = x - self.anchor[0]
        dy = y - self.anchor[1]
        cosine = math.cos(self.anchor[2])
        sine = math.sin(self.anchor[2])
        return cosine * dx + sine * dy, -sine * dx + cosine * dy


REMOTE_MQTT_STREAM = r"""
import paho.mqtt.client as mqtt

client = mqtt.Client()
client.on_message = lambda _client, _userdata, message: print(
    message.payload.decode(), flush=True
)
client.connect("127.0.0.1", 1883)
client.subscribe("uagv/v2/easygo/313757/state")
client.loop_forever()
"""


def build_api_source(
    api_port: int,
    method: str,
    path: str,
    body: dict[str, Any] | None,
) -> str:
    encoded_body = json.dumps(body).encode().hex() if body is not None else None
    return "\n".join(
        [
            "import json, urllib.error, urllib.request",
            f"url = 'http://127.0.0.1:{api_port}{path}'",
            f"data = {encoded_body!r}",
            "payload = bytes.fromhex(data) if data is not None else None",
            f"request = urllib.request.Request(url, data=payload, method={method!r})",
            "request.add_header('Content-Type', 'application/json')",
            "try:",
            "    response = urllib.request.urlopen(request, timeout=10)",
            "    print(json.dumps({'status': response.status, 'body': response.read().decode()}))",
            "except urllib.error.HTTPError as error:",
            "    print(json.dumps({'status': error.code, 'body': error.read().decode()}))",
        ]
    )


class RemoteRobot:
    def __init__(self, host: str, container: str = "easygo", api_port: int = 18089) -> None:
        if not re.fullmatch(r"[A-Za-z0-9._-]+", host):
            raise ValueError("invalid SSH host")
        if not re.fullmatch(r"[A-Za-z0-9._-]+", container):
            raise ValueError("invalid container name")
        self.host = host
        self.container = container
        self.api_port = api_port

    def api(self, method: str, path: str, body: dict[str, Any] | None = None) -> dict[str, Any]:
        if method not in {"GET", "POST"} or not path.startswith("/api/v6/"):
            raise ValueError("invalid API request")
        source = build_api_source(self.api_port, method, path, body)
        result = self._python(source, timeout=20)
        envelope = json.loads(result.stdout)
        if envelope["status"] >= 400:
            raise RuntimeError(f"HTTP {envelope['status']}: {envelope['body']}")
        return json.loads(envelope["body"])

    def status(self) -> dict[str, Any] | None:
        return self.api("GET", "/api/v6/tasks/current/status").get("data")

    def start(self, task_id: str) -> dict[str, Any]:
        return self.api("POST", "/api/v6/tasks/current/start", {"task_id": task_id})

    def stop(self) -> None:
        try:
            self.api("POST", "/api/v6/tasks/current/stop")
        except RuntimeError as error:
            if "HTTP 409" not in str(error):
                raise

    def states(self, timeout: int) -> subprocess.Popen[str]:
        encoded = base64.b64encode(REMOTE_MQTT_STREAM.encode()).decode()
        command = (
            f"docker exec {self.container} timeout --signal=TERM {timeout}s python3 -u -c "
            f"\"import base64;exec(base64.b64decode('{encoded}'))\""
        )
        return subprocess.Popen(
            ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", self.host, command],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )

    def sample_state(self, timeout: int = 10) -> dict[str, Any]:
        process = self.states(timeout)
        try:
            assert process.stdout is not None
            deadline = time.monotonic() + timeout
            line = readline_until(process.stdout, deadline)
            if line:
                return json.loads(line)
            raise RuntimeError("timed out waiting for VDA5050 state")
        finally:
            process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()

    def wait_stopped(self, timeout: int = 30) -> None:
        deadline = time.monotonic() + timeout
        stable = 0
        while time.monotonic() < deadline:
            state = self.sample_state()
            velocity = state.get("velocity", {})
            stopped = abs(float(velocity.get("vx", 0.0))) < 0.01 and abs(
                float(velocity.get("omega", 0.0))
            ) < 0.02
            stable = stable + 1 if stopped else 0
            if stable >= 2:
                return
        raise RuntimeError("vehicle did not confirm stopped")

    def _python(self, source: str, timeout: int) -> subprocess.CompletedProcess[str]:
        encoded = base64.b64encode(source.encode()).decode()
        command = (
            f"docker exec {self.container} python3 -c "
            f"\"import base64;exec(base64.b64decode('{encoded}'))\""
        )
        return subprocess.run(
            ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", self.host, command],
            check=True,
            capture_output=True,
            text=True,
            timeout=timeout,
        )


def motion_status(state: dict[str, Any]) -> str | None:
    for action in state.get("actionStates", []):
        if action.get("actionType") == "motionTestPath":
            return action.get("actionStatus")
    return None


def task_is_active(status: dict[str, Any] | None) -> bool:
    return bool(status) and status.get("phase") in {
        "starting",
        "running",
        "pausing",
        "paused",
        "resuming",
        "stopping",
    }


def has_preactivation_terminal(state: dict[str, Any], thresholds: Thresholds) -> bool:
    analyzer = TrialAnalyzer(loops_required=1, thresholds=thresholds)
    analyzer.feed(state)
    return analyzer.terminal_reason is not None


def check_preflight(state: dict[str, Any], thresholds: Thresholds) -> None:
    safety = state.get("safetyState", {})
    pose = state.get("agvPosition", {})
    if (
        safety.get("eStop", "NONE") != "NONE"
        or bool(safety.get("fieldViolation"))
        or not pose.get("positionInitialized", False)
        or float(pose.get("localizationScore", 0.0)) < thresholds.min_localization_score
    ):
        raise RuntimeError("preflight failed: vehicle state is not safe and ready")
    velocity = state.get("velocity", {})
    if abs(float(velocity.get("vx", 0.0))) >= 0.01 or abs(float(velocity.get("omega", 0.0))) >= 0.02:
        raise RuntimeError("preflight failed: vehicle is moving")


def monitor_trial(
    robot: RemoteRobot,
    loops: int,
    timeout_s: int,
    thresholds: Thresholds,
    line_length_m: float,
) -> TrialReport:
    analyzer = TrialAnalyzer(loops, thresholds, line_length_m)
    process = robot.states(timeout_s)
    deadline = time.monotonic() + timeout_s
    activated = False
    try:
        assert process.stdout is not None
        while time.monotonic() < deadline:
            line = readline_until(process.stdout, deadline)
            if line is None:
                break
            if not line:
                if process.poll() is not None:
                    stderr = process.stderr.read() if process.stderr else ""
                    raise RuntimeError(f"MQTT state stream ended: {stderr.strip()}")
                continue
            state = json.loads(line)
            status = motion_status(state)
            if status == "RUNNING":
                activated = True
            if not activated:
                if has_preactivation_terminal(state, thresholds):
                    analyzer.feed(state)
                    return analyzer.report()
                continue
            analyzer.feed(state)
            report = analyzer.report()
            if report.terminal_reason or (
                report.loops >= loops and len(report.line_endpoints) >= loops * 2
            ):
                return report
        raise RuntimeError(f"trial timed out after {timeout_s}s")
    finally:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()


def write_report(report: TrialReport, output: Path | None) -> None:
    rendered = json.dumps(asdict(report), ensure_ascii=False, indent=2)
    print(rendered)
    if output:
        output.write_text(rendered + "\n", encoding="utf-8")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run and score EasyGo motionTestPath trials")
    parser.add_argument("--host", default="sz-2")
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("status")
    subparsers.add_parser("stop")
    trial = subparsers.add_parser("trial")
    trial.add_argument("--task-id", required=True)
    trial.add_argument("--loops", type=int, default=12)
    trial.add_argument("--timeout", type=int, default=360)
    trial.add_argument("--line-length", type=float, default=3.0)
    trial.add_argument("--output", type=Path)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    robot = RemoteRobot(args.host)
    if args.command == "status":
        print(json.dumps(robot.status(), ensure_ascii=False, indent=2))
        return 0
    if args.command == "stop":
        robot.stop()
        robot.wait_stopped()
        return 0

    thresholds = Thresholds()
    if task_is_active(robot.status()):
        raise RuntimeError("refusing to replace an active task; run stop first")
    check_preflight(robot.sample_state(), thresholds)
    robot.start(args.task_id)
    try:
        report = monitor_trial(robot, args.loops, args.timeout, thresholds, args.line_length)
    except (Exception, KeyboardInterrupt):
        robot.stop()
        robot.wait_stopped()
        raise
    write_report(report, args.output)
    if report.terminal_reason == "external_stop":
        return 20
    if report.stop_required:
        robot.stop()
        robot.wait_stopped()
        return 30
    if not report.accepted:
        robot.stop()
        robot.wait_stopped()
        return 2
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RuntimeError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(3)
