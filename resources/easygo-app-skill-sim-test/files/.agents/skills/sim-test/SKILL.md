---
name: sim-test
description: Use when running SROS simulation pytest suites or single cases, primarily on a remote simulation host, for smoke, core, nightly, or targeted domain debugging.
argument-hint: "[smoke|core|nightly|<test-path>] [ssh-host]"
---

# 仿真测试执行

运行 SROS 仿真 pytest 测试，**默认以远程仿真机为主场景**。  
本地执行只作为补充路径。

## 主路径

### 1. 远程分级测试

适合 `smoke` / `core` / `nightly`：

```bash
cd simulation
./scripts/deploy.sh remote sim-test <host> [smoke|core|nightly] [--instance=<name>] [--no-sync] [pytest args...]
```

示例：

```bash
./scripts/deploy.sh remote sim-test dev-gpu-5060
./scripts/deploy.sh remote sim-test dev-gpu-5060 smoke
./scripts/deploy.sh remote sim-test dev-gpu-5060 smoke --instance=exp1
./scripts/deploy.sh remote sim-test dev-gpu-5060 core --no-sync -k navigation
./scripts/deploy.sh remote sim-test dev-gpu-5060 nightly -x
```

说明：

- 未指定级别时默认 `smoke`
- `--no-sync` 仅在你确认远端代码已同步时使用
- 这是 MR 门禁 / 回归验证的主入口

### 2. 远程单文件 / 单用例

当用户要跑某个具体 case，优先直接在远程仿真机上执行：

```bash
ssh <host> "cd ~/sros-sim/instances/<name>/src/simulation && python3 -m pytest <test-path> -v -p no:cacheprovider"
```

示例：

```bash
ssh dev-gpu-5060 "cd ~/sros-sim/instances/main/src/simulation && python3 -m pytest tests/test_navigation.py::TestNavigationSmoke::test_path_navigation_forward_1500mm -v -p no:cacheprovider"
ssh dev-gpu-5060 "cd ~/sros-sim/instances/main/src/simulation && python3 -m pytest tests/test_obstacle.py -v -p no:cacheprovider"
```

这是排查 flaky、做 U1 spike、复现单个失败最常用的路径。

## 次路径

只有在用户明确要求本地，或远程主机不可用时，才使用本地测试：

```bash
cd simulation
./scripts/deploy.sh sim-test [smoke|core|nightly] [pytest args...]
python3 -m pytest tests/<...> -v -p no:cacheprovider
```

## 测试级别

| Marker | 含义 | 时间预算 | 典型用途 |
|--------|------|----------|----------|
| `smoke` | 核心冒烟 | <10min 总 | MR 前快速确认 |
| `core` | 核心合约 | <30min 总 | Nightly 核心回归 |
| `nightly` | 扩展合约 | <60min 总 | 全量夜跑 |

## 测试域

| 文件 | 域 | 关注点 |
|------|-----|--------|
| `tests/test_localization.py` | 定位 | 初始化定位、位姿精度 |
| `tests/test_navigation.py` | 导航 | 路径规划、到达精度 |
| `tests/test_obstacle.py` | 避障 | 动态/静态障碍物 |
| `tests/test_safety.py` | 安全 | 碰撞检测、取消恢复、急停链路 |
| `tests/test_robustness.py` | 稳定性 | repeated reset、长时运行 |

## 前置条件

- 仿真环境已启动，远程优先：`./scripts/deploy.sh remote start <host>`
- SROS 已初始化完成，Web UI 可访问
- 跑单 case 时，优先确认远端 `~/sros-sim/instances/<name>/src/simulation` 中代码已经是当前版本

## 选择规则

- 用户给出主机名、说“远程仿真机 / 远程服务器 / dev-gpu-*”时：远程执行
- 用户说“跑 smoke/core/nightly”时：优先 `deploy.sh remote sim-test`
- 用户说“跑某个 test file / 某个 test case”时：优先远程直接 `python3 -m pytest`
- 用户明确说“本地”时：走本地路径

## 远程主机继承

当用户没有显式给出 `<host>` 时，按下面规则处理：

1. 当前会话最近一次成功使用过远程仿真主机时，默认沿用该主机
2. 如果上下文里只有一个明确候选主机，例如 `dev-gpu-5060`，默认使用它
3. 如果存在多个候选主机且无法判断，先简短追问，不要猜
4. 只有在没有远程主机上下文、且用户明确说“本地”时，才切回本地测试

示例：

```text
前文刚执行过：./scripts/deploy.sh remote status dev-gpu-5060
用户随后说：$sim-test smoke
=> 默认理解为：./scripts/deploy.sh remote sim-test dev-gpu-5060 smoke

前文刚执行过：ssh dev-gpu-5060 "cd ~/sros-sim/instances/main/src/simulation && python3 -m pytest ..."
用户随后说：$sim-test tests/test_navigation.py::TestNavigationSmoke::test_path_navigation_forward_1500mm
=> 默认继续在 dev-gpu-5060 上跑该单 case
```

## 常见失败原因

1. **定位漂移 / baseline 没恢复**
   表现：初始定位置信度低、导航偏航、首跑失败后续通过

2. **Gazebo 暂停**
   表现：激光流超时、bridge 正常但行为不前进

3. **远端代码或二进制不是最新**
   表现：本地改了测试或 bridge，但远端行为和预期不一致

4. **远端 release lock 不是预期镜像**
   表现：已经构建并启动过新镜像，但 `~/sros-sim/instances/<name>/current/manifest.json` 或 `docker-compose.lock.yml` 仍指向旧 digest，测试 fixture 触发 compose recreate 时会继续使用当前 lock 中的旧镜像

5. **状态残留**
   表现：上一个 case 的 obstacle / task / emergency 污染下一个 case

## 注意事项

- `deploy.sh remote sim-test` 当前适合 tier 级测试；单 case 更适合远程直接 `python3 -m pytest`
- 在远程宿主机上，优先用 `python3 -m pytest`，不要假设全局 `pytest` 命令一定存在
- 改过 `simulation/docker/gz-sim/` 或 `simulation/bridge/` 后，先在远端代码目录用 `./scripts/build_images.sh --only=gz-sim` 构建推送新镜像，再用 `GZ_SIM_IMAGE_TAG=<new-tag> ./scripts/deploy.sh remote start <host>` 生成新的 release lock；远端运行态只认 `~/sros-sim/instances/<name>/current/` 下的 release lock（`docker-compose.lock.yml` + `compose.env` + `manifest.json`），早已去除远端 `.env`
- 验证 dev-gpu 主机是否真正使用 GPU 渲染时，不要只看 nvidia runtime 或 `Render: GPU hardware` 日志；还要确认 `~/sros-sim/instances/<name>/current/manifest.json` 记录的 `GZ_SIM_IMAGE_TAG`、`docker inspect sros-<name>-gz-sim` 的镜像 tag，并用 `nvidia-smi pmon -c 1` 看到 Gazebo/world 进程产生 GPU 负载
- 如果结果明显受状态影响，先看 `remote status` / `remote diag`，再决定是否 `remote reset`
- 如果本轮对话已经锁定远程主机，后续 smoke/core/nightly 和单 case 默认继承该主机
