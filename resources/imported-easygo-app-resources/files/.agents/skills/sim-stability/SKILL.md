---
name: sim-stability
description: Use when validating SROS simulation stability by running repeat_fanout batches across multiple remote hosts, especially 10-host nightly repeats, pass-rate checks, Gazebo GUI/headless mode selection, and failure triage.
argument-hint: "[nightly|core|smoke] [total-runs] [host-list]"
---

# 仿真稳定性批量验证

使用多台远程主机批量运行仿真 pytest，验证 SROS simulation 在重复 nightly/core/smoke 场景下的稳定性。  
默认目标是**发现真实稳定性问题**，不是追求最快完成；但正常情况下不限制并发，按 ready host 数调度，每台 host 同时最多一个 run。

## 使用场景

- 用户要求“多台主机跑 N 次 nightly/core/smoke”
- 用户说“跑一轮 100x 验证 / 100 次验证 / 100x nightly”时，默认理解为：在默认主机池中运行一轮 100 次 `nightly` fanout
- 用户要求验证仿真稳定性、flaky、长轮次回归
- 用户指定 `10.10.18.71-80`、`dev-gpu-*` 等远程仿真主机
- 用户要求 Gazebo 关闭 GUI/headless 运行

## 默认主机池

如果用户没有显式指定主机池，默认使用：

```bash
hosts="10.10.18.71,10.10.18.72,10.10.18.73,10.10.18.74,10.10.18.75,10.10.18.76,10.10.18.77,10.10.18.78,10.10.18.79,10.10.18.80"
```

只有用户明确给出其他 host 列表、单机名或要求排除某台机器时，才覆盖这个默认池。

## 默认语义

当用户说“跑一轮 100x 验证”且没有额外限定时，执行含义固定为：

- 主机池：默认 `10.10.18.71-80`
- tier：`nightly`
- 总次数：`100`
- 模式：远程多机 `repeat_fanout`
- Gazebo：默认启用 GUI；需要无头模式时显式加 `--headless`
- 并发：按 ready host 数调度，每台 host 同时最多一个 run

## 标准流程

### 1. 单机基线

大批量前先在一台代表主机跑一轮完整 tier，确认代码和环境基本可用：

```bash
cd simulation
./scripts/deploy.sh remote sim-test dev-gpu-5060 nightly --headless
# 也可指定实例（默认 main）：
./scripts/deploy.sh remote sim-test dev-gpu-5060 nightly --headless --instance=exp1
```

如果单机失败，不要直接启动多机 fanout。先定位失败用例，必要时跑单文件/单 case。

### 2. 单构建主机分发（人工流程）

在脚本/CI 支持“单构建主机，多机分发”之前，如果需要确保所有 host 使用同一份二进制，不要让每台 host 各自 `build-update`。默认使用 `dev-gpu-5060` 作为构建主机（除非用户显式指定其他机器或该机器不可用），只在该机器编译一次，再用 `remote deploy` 分发到其他仿真主机。

```bash
build_host="dev-gpu-5060"
hosts="10.10.18.71,10.10.18.72,10.10.18.73,10.10.18.74,10.10.18.75,10.10.18.76,10.10.18.77,10.10.18.78,10.10.18.79,10.10.18.80"

cd simulation
./scripts/deploy.sh remote build-update "$build_host"

IFS=',' read -r -a host_array <<< "$hosts"
for host in "${host_array[@]}"; do
  if [ "$host" != "$build_host" ]; then
    ./scripts/deploy.sh remote deploy "$build_host" "$host"
  fi

  # 若该 host 的远端测试 venv 可能不存在，先跑一轮 smoke 让
  # remote sim-test 同步 tests/scripts 并创建 .venv。
  # repeat_fanout 默认使用实例 fanout（与交互式 main 分域），可用 --instance=<name> 覆盖。
  ./scripts/deploy.sh remote sim-test "$host" smoke --headless
done
```

之后启动 fanout 时使用 `--skip-prepare`，避免再触发每台 host 的 `build-update`：

```bash
label="nightly-100-single-build-$(date +%Y%m%d-%H%M%S)"
./scripts/repeat_fanout.sh \
  --hosts "$hosts" \
  --tier nightly \
  --total 100 \
  --skip-prepare \
  --label "$label"
```

注意：

- 这是临时人工流程；后续如需 CI 化，应在脚本中显式建模 `build_host` 和分发步骤。
- 不要通过环境变量强行改写 `rsync_build` 默认语义；构建一致性应由“单构建主机 + deploy 分发”保证。
- `remote deploy` 会复制二进制并校验 MD5；`remote sim-test` 负责同步测试源码和准备远端 pytest venv。

### 3. 多机批量

默认 10 台主机池：

```bash
hosts="10.10.18.71,10.10.18.72,10.10.18.73,10.10.18.74,10.10.18.75,10.10.18.76,10.10.18.77,10.10.18.78,10.10.18.79,10.10.18.80"
label="nightly-100-gui-$(date +%Y%m%d-%H%M%S)"

cd simulation
./scripts/repeat_fanout.sh \
  --hosts "$hosts" \
  --tier nightly \
  --total 100 \
  --prepare-mode start \
  --label "$label"
# repeat_fanout 默认实例为 fanout（与交互式默认实例 main 分域），可用 --instance=<name> 覆盖。
# 同一主机并行两路 fanout 时，用不同实例名（例如 --instance=fanout-a 和 --instance=fanout-b）。
```

说明：

- `repeat_fanout` 会执行 host prepare/probe，并在 fanout 前把 Gazebo GUI 调整到本轮要求的模式；默认开启 GUI。
- 只有确认远端代码和环境已经同步时，才加 `--skip-prepare`。
- 如果刚更新过 gz-sim 镜像，启动 fanout 前必须确认 `~/sros-sim/instances/<name>/current/manifest.json` 和 `docker-compose.lock.yml` 已经指向新 digest。`remote sim-test` 只读取 current release lock；如果 lock 仍是旧 digest，测试 fixture 里的 compose recreate 会继续拉起旧 `sros-<name>-gz-sim` 镜像。
- 只有明确需要缩短单 run 判死时间时，才加 `--per-run-timeout <seconds>`。
- fanout 默认走 CI/supervisord restart 语义；需要关闭 Gazebo GUI 时显式加 `--headless`，本地调试才考虑 `--no-ci`。

## 监控进度

artifact 在：

```bash
simulation/artifacts/<label>
```

查看累计结果：

```bash
artifact="simulation/artifacts/<label>"
printf 'results='; find "$artifact" -name result.json | wc -l
printf 'passed='; rg -l '"status": "passed"' "$artifact"/run-*/result.json 2>/dev/null | wc -l
printf 'failed='; rg -l '"status": "failed"' "$artifact"/run-*/result.json 2>/dev/null | wc -l
```

如果进程仍在运行，优先持续轮询现有 session，不要重复启动同一批次。
fanout 的完整 stdout 以本地 `simulation/artifacts/<label>/run-*/stdout.log` 为准；远端 `~/sros-sim/instances/<name>/src/simulation/artifacts/run-*` 目录不一定有 `stdout.log`，通常只保留 pytest report / runtime metrics 等文件。

## 失败处理

### 100x 整轮验证

当用户要求“跑一轮 100x 验证”或其他明确的整轮批量验证时，**不要因为中途出现失败而终止整轮执行**。中途失败只做记录：

- 记录失败 run、host、失败用例、`failure_reason` 和 artifact 路径。
- 继续让 `repeat_fanout` 跑满用户要求的总次数。
- 等整轮结束后，再统一汇总失败模式并决定是否修复和重跑。
- 只有基础设施级故障导致无法继续调度（例如主机池全部不可用、脚本自身崩溃）时，才停止并先处理调度/环境问题。

### 定位失败

整轮结束后按 run 定位：

```bash
artifact="simulation/artifacts/<label>"
for d in "$artifact"/run-*; do
  printf '\n### %s\n' "$(basename "$d")"
  rg -n "FAILED|ERROR|FAILURES|ERRORS|AssertionError|Timed out|Deadline Exceeded|Emergency not triggered|APE RMSE|RPE RMSE" "$d/stdout.log" | tail -120 || true
done
```

再看 `result.json` 区分类型：

- `pytest_failed`：真实测试失败，优先看 pytest traceback 和失败用例。
- `timeout` / `execution_failed`：先看 SSH、容器、Gazebo bridge、RTF、host 是否不稳定。
- `no_available_hosts`：看 `hosts.json` 的 prepare/probe 失败原因。

修复后先跑相关单测和单机 nightly，再重新跑多机批量。

## 结束检查

100% 通过后仍要做落盘和远端残留检查：

```bash
artifact="simulation/artifacts/<label>"
printf 'results='; find "$artifact" -name result.json | wc -l
printf 'passed='; rg -l '"status": "passed"' "$artifact"/run-*/result.json 2>/dev/null | wc -l
printf 'failed='; rg -l '"status": "failed"' "$artifact"/run-*/result.json 2>/dev/null | wc -l
printf 'missing_xml='; find "$artifact" -mindepth 1 -maxdepth 1 -type d -name 'run-*' | while read -r d; do test -f "$d/pytest-report.xml" || printf '%s\n' "$d"; done | wc -l
```

检查远端没有 pytest 残留，且 Gazebo GUI 关闭：

```bash
for host in 10.10.18.71 10.10.18.72 10.10.18.73 10.10.18.74 10.10.18.75 10.10.18.76 10.10.18.77 10.10.18.78 10.10.18.79 10.10.18.80; do
  printf '\n### %s\n' "$host"
  timeout 15 ssh -o BatchMode=yes -o ConnectTimeout=5 "$host" \
    'pgrep -af "[p]ytest tests/" || true; docker exec sros-fanout-gz-sim supervisorctl status gazebo-gui gazebo-server gz-sim-bridge 2>/dev/null || true'
done
```

期望：

- 无 pytest 残留进程
- `gazebo-gui` 为 `STOPPED`
- `gazebo-server` 和 `gz-sim-bridge` 为 `RUNNING`

## 常见判断

- 单机不过：不要跑多机，先修功能或环境。
- 多机少量 host probe 失败：先看 `hosts.json`，不要假设是用例失败。
- RTF 偏低（尤其 `<0.8`）：优先检查 `gazebo-gui` 是否仍为 `RUNNING`；GUI 开启会大幅拉低 RTF，先执行 `./scripts/deploy.sh remote gui <host> off` 并复查 supervisor 状态。
- dev-gpu 主机验证 GPU 渲染时，至少复查三项：`~/sros-sim/instances/<name>/current/manifest.json` 记录的 `GZ_SIM_IMAGE_TAG`（远端运行态身份来自 release lock，已无远端 `.env`）、`docker inspect sros-<name>-gz-sim` 的镜像 tag、`nvidia-smi pmon -c 1` 是否出现 Gazebo/world 进程。只看到 `Render: GPU hardware` 或 nvidia runtime 不足以证明测试期间一直使用了新 GPU 渲染链路。
- safety/collision/emergency 连续失败：优先怀疑 Gazebo baseline reset、ContactSensor、bridge 状态。
- localization/navigation 漂移：优先看 baseline 恢复、RTF、定位初始化和上个 case 状态残留。
- 用户要求“不要 commit”时，验证和修复完成后只报告结果，不提交。
