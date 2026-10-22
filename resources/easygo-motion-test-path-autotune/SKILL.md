---
name: easygo-motion-test-path-autotune
description: Use when an EasyGo motionTestPath line/rotate task needs repeated controller tuning and live validation on a named robot until it runs smoothly, including overshoot, pullback, heading error, oscillation, or speed regressions.
---

# EasyGo MotionTestPath Autotune

Run a single-variable, evidence-driven loop from source change through live robot acceptance. Keep the requested task running after success.

## Preconditions

- Require explicit authorization to deploy and start the physical robot.
- Use an isolated clean VDA5050 worktree on the existing MR branch. Never build `src/Vda5050_ROS2` when the candidate lives elsewhere.
- Record the host, saved task ID, path length, baseline SHA/package, and current task state.
- Preserve action semantics: `loop_count=0` is infinite and path coordinates remain in the action-start frame. Do not reintroduce action-level collision avoidance unless requested.

## Closed Loop

1. Read current source, MR diff, installed package, backend status, and one MQTT state. Form one root-cause hypothesis from measured evidence.
2. Add one failing focused test, make the minimum code/config change, and pass the focused plugin tests.
3. Commit and push the candidate to the existing MR. Update its description with the hypothesis and later trial metrics. Do not wait for CI unless requested.
4. Stop the active task and confirm two consecutive zero-velocity samples:

```bash
python3 scripts/trial_monitor.py --host "$ROBOT" stop
```

5. Build/package, then deploy from the exact worktree:

```bash
python3 scripts/deploy_worktree.py --workspace "$EASYGO" --source "$VDA_WORKTREE" --host "$ROBOT" --mode package
python3 scripts/deploy_worktree.py --workspace "$EASYGO" --source "$VDA_WORKTREE" --host "$ROBOT" --mode deploy
```

Verify the installed version, `ros2-launch`, the VDA/MQTT child process, launch logs, and HTTP status. Parent supervisor state alone is insufficient.

6. Start and score 12 complete loops through the backend API:

```bash
mkdir -p /tmp/motion-test-path-trials
python3 scripts/trial_monitor.py --host "$ROBOT" trial \
  --task-id "$TASK_ID" --loops 12 --line-length 3 \
  --output "/tmp/motion-test-path-trials/$(git -C "$VDA_WORKTREE" rev-parse --short HEAD).json"
```

Acceptance requires P95 endpoint position error <= 0.03 m, P95 endpoint heading error <= 5 degrees, maximum pullback <= 0.02 m, median line peak speed >= 0.76 m/s, no localization/safety/runtime failure, and all 12 loops.

7. On metric rejection, the script stops the task. Revert the failed candidate, push the MR, re-investigate, and repeat with one new hypothesis. Never stack an unproven tuning change. After three failed hypotheses, reread the controller geometry end to end before the next edit.

## Termination

- Success: leave the accepted infinite task running; report MR SHA, package version, and trial JSON.
- Human pause/cancel/stop: do not restart or deploy; end the loop.
- E-stop, safety-field violation, lost localization, or unsafe motion: stop immediately and end the loop.
- Candidate action/test/build/runtime failure: reject that candidate and continue from evidence.

## Scripts

- `scripts/trial_monitor.py`: HTTP control, MQTT sampling, safety gate, segmentation, metrics, and accept/reject exit status.
- `scripts/deploy_worktree.py`: package/deploy through EasyGo `script/deploy-dev.py` while forcing the selected VDA5050 worktree.
