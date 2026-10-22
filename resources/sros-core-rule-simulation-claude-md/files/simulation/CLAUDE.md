# 仿真系统开发指南

> 本文件是 `simulation/` 子系统的开发索引与关键约束。
> 日常使用见 [README.md](README.md)。
> 整体设计见 [docs/architecture-overview.md](docs/architecture-overview.md)。
> 常见问题与已知陷阱见 [docs/troubleshooting.md](docs/troubleshooting.md)。

## 文档导航

- [README.md](README.md)
  - 日常启动、更新、测试命令
- [docs/architecture-overview.md](docs/architecture-overview.md)
  - 整体架构、分层职责、执行流、工具选择
- [docs/test-development-guide.md](docs/test-development-guide.md)
  - 仿真测试分层、用例表达、注释、fixture 与 helper 开发规范
- [docs/troubleshooting.md](docs/troubleshooting.md)
  - 常见问题、部署陷阱、bridge/定位/避障排查
- [docs/reset-audit-checklist.md](docs/reset-audit-checklist.md)
  - reset 相关审计背景
- [tests/ASSERTION_RULES.md](tests/ASSERTION_RULES.md)
  - 新增或修改仿真测试时必须遵循的断言与评审门禁

## 快速结论

- `pytest tests/` 和 `./scripts/deploy.sh sim-test` 是唯一主线验证入口
- 新增或修改 `tests/` 下的测试、fixture 与测试 helper 必须遵循
  [仿真测试开发规范](docs/test-development-guide.md)与
  [仿真测试断言规则](tests/ASSERTION_RULES.md)；违反规则属于评审阻塞问题
- 回归主线以 e2e 为主：`pytest -m regression`（或 `-m "e2e and smoke"`）跑 gulf 取放货 + oasis 顶升搬运；`-m e2e` 只选真 e2e 场景，不含单功能用例
- 单功能定位：`pytest tests/live/functional/<域>`（`-m regression` 不选；`--sim-suite` 按 `sim_level` 仍会选中）；ONES
  测试计划转换用例位于 `tests/protobuf_test/<域>`。
- 静态秒级：`pytest tests/static`（无需仿真环境，任意机器可跑）
- suite / tier 级多机重复执行使用 `./scripts/repeat_fanout.sh`
- single-test 高样本波动测量继续使用 `./scripts/flaky_measure.py`
- 本机无编译工具链；编译必须在 Docker 容器或远程主机完成
- 仿真镜像构建推送与部署策略详见 [README.md](README.md) 的"仿真镜像管理"一节
- 改 sros 代码/profile/world 用 dev 模式（`deploy.sh dev up/shell`）：挂宿主源码直接跑，**不过镜像 lock 闸门**，编译完重启 sros 即可，详见 [README.md](README.md)
- `sros-sim-runtime` 镜像内包含 SROS runtime `/sros` 基线；远程实例默认 `clean`，容器重建恢复干净基线
- 长期手工调试使用 `--state-mode=persistent`，只持久化 DB、地图和机器人图标；`start/build-update` 前自动备份并保留最近 2 份，`sim-test` 禁止使用该模式
- `simulation/profile_loader`、`profiles`、`worlds`、`maps` 是仿真运行输入，通过源码目录只读挂载进 runtime 容器，不纳入 `SROS_RUNTIME_IMAGE_TAG`
- Gazebo 位姿 reset 后，必须重新 `stop_location + start_location`

## 命令入口

- 日常启动、更新、验证命令：见 [README.md](README.md)
- 整体入口分工与适用场景：见 [docs/architecture-overview.md](docs/architecture-overview.md)

## 核心架构摘要

### 运行时栈

- `sros-gz-sim`
  - Gazebo Harmonic
  - `gz-sim-bridge`
  - VNC / noVNC
- `sros-sim-runtime`
  - `sros` 主进程
  - Web UI
  - WebSocket Agent
- `sros-rerun-viewer`
  - Rerun 可视化

### 双通道设计

- **Control Plane**
  - `scripts/lib/sros_client.py`
  - 通过 TCP protobuf 驱动 SROS 行为
- **Truth Plane**
  - `tests/fixtures/gz_bridge.py`
  - 通过 gRPC 直接读取 Gazebo 真值

测试通过这两条通道实现“驱动系统”和“独立校验结果”的分离。

### pytest 主线框架

- `tests/conftest.py`
  - fixture 装配、session 生命周期、失败诊断
- `tests/fixtures/sim_env.py`
  - reset、重启、定位恢复、`deterministic_start`
- `tests/fixtures/state_contract.py`
  - session/test 级状态契约
- `tests/fixtures/world_registry.py` + `world_manager.py`
  - world/map 解析与切换
- `tests/fixtures/robot_driver.py` + `action_tasks.py`
  - 基础能力层；分别提供导航、动作和执行机构入口，统一 task id、提交重试、终态校验和失败诊断
- `tests/fixtures/gz_bridge.py` + `cleanup.py`
  - 复用同一 Gazebo 实体接口完成相对位姿换算、创建、移动和框架级清理，不再增加平行实体管理类
- `worlds/models/<robot_model>/model.sdf` + resolved profile
  - 产品车型物理结构和运行参数的事实源；SDF 声明几何、关节和传感器安装，profile 声明该次运行采用的设备与参数
- `tests/fixtures/*_simulator.py`
  - 可选车型适配器；仅在某车型存在通用 `RobotDriver`、`gz_bridge` 和配置无法直接表达的稳定行为或组合真值时增加
- `tests/assertions/`
  - 业务断言层；根据 `gz_bridge` 或适用车型适配器提供的独立真值判断验收结果，不执行机器人或 Gazebo 行为
- `tests/fixtures/*_pickplace.py` 等参数模块
  - 只声明站点、Action ID 和业务参数，不执行行为

## 关键约束

### 验证主线

- 新增协议能力：先进入 `scripts/lib/sros_client.py`
- 新增长期验证能力：进入 `simulation/tests/`
- 框架优化以可读性、可维护性和扩展性为目标；对象数量和命名是当前领域建模结果，不是目标本身
- 用例直接组合 `robot.navigation`、`robot.actions`、`robot.fork` 等基础能力，并保留业务参数和断言
- YAML profile 解析后的 `robot_model` 严格对应一个真实产品型号，并选择其 SDF；同一车型的场景、传感器、world 或现场问题 profile 复用这份物理模型
- SDF/model config 是车型几何、关节、传感器安装等物理事实源，resolved profile 是本次运行设备和业务参数的事实源；禁止把这些值复制成 Python simulator 常量
- `RobotDriver` 是跨车型的 SROS Control Plane capability 容器；fixture 名保持为 `robot`，不得按具体车型重复实现控制驱动
- 产品车型适配器不是必备的一一对应物。只有确有车型独有且可复用的行为或组合真值时才增加；一旦增加，必须只对应一个真实产品型号并使用产品名，禁止按测试场景、传感器或用例主题命名
- 车型之外的实体操作直接扩展、复用 `gz_bridge`，场景专属编排留在用例附近，禁止为已有实体 API 再建平行管理类
- `robot`、`gz_bridge` 和可选车型适配器由 pytest fixture 注入；禁止 live test 直接构造这些基础设施对象
- 禁止让车型适配器下发导航/动作或暴露业务 `expect_*`/`assert_*`；也禁止用单个 `run_*_flow()` 隐藏完整场景
- 用例文件必须让前置、动作和紧邻动作的验收点直接可见；简单状态使用原生 `assert`，跨用例复用的真值判定进入 `tests/assertions/`
- 每个 `test_*` 函数必须有至少一句简体中文 docstring，说明前置、动作和预期结果；阶段注释和业务语义注释也使用简体中文，协议字段、API 名称、模型名和标准专有名词可以保留原文
- 多机 fanout repeat：使用 `./scripts/repeat_fanout.sh --hosts ... --tier <smoke|core|nightly> --total N`
- `repeat_fanout.sh` 默认启用 Gazebo GUI，并透传 pytest `--ci`，保证内部重启走 supervisord 路径；需要无头模式时显式加 `--headless`
- `repeat_fanout.sh` 不做 retry；任一 run 失败即计入失败样本并返回非零退出码
- `repeat_fanout.sh` 默认按同构 host 池处理，不做异构校准
- 单个 case 的高样本度量：继续使用 `./scripts/flaky_measure.py`
- 一次性诊断：优先新增独立小工具

### 构建与部署

- 本机无编译工具链
- 推荐工作流：
  - 改 sros 代码/profile/world：dev 模式，见 [README.md](README.md)「dev 模式」
  - 日常启动与测试：见 [README.md](README.md)
  - 远程编译部署：见 [README.md](README.md) 中远程编译与验证部分
  - 镜像构建推送：见 [README.md](README.md) 中“仿真镜像管理”部分
- `gz-sim-bridge` 代码变更后：
  - `build-update` 不够（只更新 sros 二进制，不触及 bridge 镜像）
  - 必须先 `./scripts/build_images.sh` 推新 tag、`--write-lock` 刷新并提交 `sim-images.lock.json`，再 `remote start <host>`
  - 未刷新 lock 时部署被 drift 闸门拒绝（`SIM_ALLOW_IMAGE_DRIFT=1` 强制跳过）；镜像版本事实源是入库 `sim-images.lock.json`，部署优先级：显式 tag > 入库 lock > 继承远端
  - 本地 `deploy.sh start`/`sim-test`/`world` 也从入库 lock 注入三镜像 tag 并过 drift 闸门，`.env` 不再配镜像 tag（唯一来源是 lock）；runtime 未发布时 fail-fast 提示先 `runtime-image`
  - lock hash 只算已跟踪/staged 文件（可复现、不受未跟踪临时文件影响）；新增文件须 `git add` 才进 hash。`remote start` 在入库 lock 与远端 current 不一致时自动刷新 release lock
  - hash 输入是 `simulation/bridge` + `modules/gz_sim/proto` + `simulation/docker/gz-sim`，**不含 `worlds/`**：镜像内 `/app/worlds` 永远被挂载的源码覆盖，加场景/改货架模型不该要求重推 Gazebo 镜像
  - 未推送时 `remote start` 会在 pull 阶段失败，不做本地 build 回退
- runtime 镜像发布同样遵循 Nexus 端口分离：构建/compose tag 使用 `nexus3.sr:8060`，推送前 retag 到 `nexus3.sr:8061` 并从写端口 push
- 本地与远程是并列执行域：本地 `runtime-image` 只使用本地 Docker；`remote build-update/deploy` 只在用户指定远端执行，失败时不自动切换到本地或其他远端

### 配置约束

| 配置项 | 期望值 | 说明 |
|--------|--------|------|
| `gz_sim.enable` | `1` | 启用 bridge（仍存于 DB） |
| `gz_sim.bridge_address` / `bridge_port` | env `GZ_BRIDGE_HOST/PORT` 优先，DB fallback | 跨容器地址由 docker-compose 注入（env），真机部署回退到 DB |
| `rerun_bridge.grpc_address` | DB（entrypoint 重置为 `127.0.0.1:9876`） | C++ 尚未消费；启用时仿 gz_sim 模式加 env-first 读取 |
| `debug.enable_sros_native_debug` | `0` | 否则激光模块不创建 |
| `function.basic.control_unit` | `1` | 启用 `GzSimSrcSdk` |
| `nav.enable_free_nav` | `1` | 启用自由导航 |

### 车型

- 默认 `oasis_600e_robot`（差速）；新增 `gulf_mp15`（单舵轮叉车，`chassis.type=5`），跑 gulf 用 `--profile gulf_mp15` + 地图 `gulf_auto_test_m3`。
- 单舵轮**可原地旋转**，但旋转中心在后轴（两从动轮）中心、恰与 GT 参考点重合 → 纯旋转位移≈0、与差速车不可区分，故 gulf e2e 的车型差异判据用「行进转向最小转弯半径」而非纯旋转位移；理由见 `tests/live/e2e/gulf/test_gulf_pick_place_flow.py` docstring。
- **gulf 固有几何参数禁止为"调绿测试"随意改**：`fork_end_coordinate=-0.17`（叉根）、`fork_arm_length=1.13`、栈板 deck 进深 `1.0`、`card_zero_info` length `1.0` 是真机 NCB15 真值、互为同一物理量的不同载体，改一处须同步全部，且**不得按动作分别覆盖**；仿真未联合标定的偏差只在 bridge 测距锚点与 `deviate_distance_x` sanity 门代偿。详见 [docs/architecture-overview.md](docs/architecture-overview.md) Gulf 章节「固有几何参数」。

### 单位约定

| 数据结构 | 单位 |
|----------|------|
| `sros::core::Pose` (x, y, yaw) | m, rad |
| `sros::core::Velocity` / `SRCState` | mm/s, mrad/s |
| `setVelocity()` 入参 | mm/s, mrad/s |
| `Pose.timestamp()` | μs (steady_clock) |

## 关键入口

- gRPC proto：`modules/gz_sim/proto/gz_sim.proto`
- SROS TCP 协议适配：`simulation/scripts/lib/sros_client.py`
- pytest 主线入口：`simulation/tests/conftest.py`
- world / map 注册：`simulation/tests/fixtures/world_registry.py`

## 深入主题

- 整体设计与用法总览：见 [docs/architecture-overview.md](docs/architecture-overview.md)
- 常见问题与已知陷阱：见 [docs/troubleshooting.md](docs/troubleshooting.md)
- reset 相关背景：见 [docs/reset-audit-checklist.md](docs/reset-audit-checklist.md)
