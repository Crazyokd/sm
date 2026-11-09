# AGENTS.md

本仓库的开发指南（SSoT），适用于所有 AI 编码助手（Codex、Claude Code 等）以及人类开发者。

## 架构概览

入口 `main.cpp`，三层架构：**模块系统 + 消息总线 + SDK 抽象**

- **模块系统**：基类 `Module`（`core/module/module.h`），`ModuleManager` 注册并启动 40+ 模块，每模块独立线程运行 `run()`
- **消息总线**：`MsgBus` 单例（`core/module/msg_bus.h`），Pub/Sub 模式，50+ 消息类型（`core/msg/base_msg.h`），`dispatch()` 主循环永不返回
- **SDK 抽象**：`SrcSdk`（`src/sdk/src_sdk.h`）为基类，真实硬件走串口，仿真走 `GzSimSrcSdk`（gRPC → Gazebo），`main.cpp::init()` 按模式选择

## 关键目录

```
main.cpp                程序入口（模块注册、SDK 选择）
core/module/            Module、ModuleManager、MsgBus 框架
core/db|settings/       SQLite 数据库 + Settings 配置单例
modules/network/        TCP Protobuf 服务器（network_module.cpp 命令分发）
protocol_adapter/       华为 LDM 协议适配器（LDM ↔ adapter ↔ SROS 5001 protobuf）
modules/gz_sim/         Gazebo 仿真 gRPC 客户端
src/sdk/                SrcSdk 基类与串口驱动
cfg/sros-resources/protobuf/  协议定义（main.proto）
simulation/       仿真系统（Gazebo bridge + 部署 + 验证脚本）
web/ui-server/          Web UI Server（启动时必须 cd 到此目录）
```

## 编程原则

严格遵循 SOLID、KISS、DRY、YAGNI 原则：

- 仅实现当前明确所需的功能，不预留未来特性
- 追求极致简洁，拒绝不必要的复杂性
- 先读后写，理解现有代码再修改
- 基于事实而非猜测，充分使用工具收集信息
- 遇到缺陷或回归问题时，优先先写能复现问题的最小单测/回归测试，再写代码修复并让该测试通过；确实不适合自动化测试时，说明原因
- 代码注释语言与现有代码库保持一致（中文项目用中文注释）

## 文档规范

随仓库版本管理的文档（`AGENTS.md` / `README.md` / `REVIEW.md` / `doc/` / `docs/` / `simulation/` 下文档等）遵循「只保留代码/测试反推不出、或重建成本过高的内容」：

- **该留**：关键决策的「为什么」与取舍、被否方案、背景约束（代码反推不出）；高频且反推费力的操作信息（构建/测试/运行命令、目录结构、约定）。
- **不该留**：逐行复述代码 what/how 的；无人读 / 无 owner 的；会随代码漂移又无法同步的；与 README/其它文档重复的（保留单一 canonical，其余改成链接）；通用废话、AI 人格 prompt、临时笔记（进会话/issue，不进永久文档）。
- **放哪**：与所描述的代码就近共置、进同一 PR；给人看的 why/决策/背景与给 AI 的命令/结构/约定按受众分载体；给 AI 的稳定规则进本文件并保持小、引用别复制。
- **维护**：每份文档有 owner，随改代码的同一 PR 同步；过时即删除，或显式标 `superseded` / `已失效` 并指向新位置——本仓沿用可更新文档（`doc/`、`docs/brainstorms/` 等），不强制 ADR 不可变。

## 构建与测试

- CMake 3.21+ / Ninja，代码规范 C++17，预设见 `CMakePresets.json`
- **本机无编译工具链，所有构建/测试操作必须在容器中执行**
- 依赖管理：Conan（`conan/arch_lib/`、`conan/public_lib/`）

### 进入容器

AI 助手先用 `uname -n` 自检：主机名包含 `sros-dev-env` 即在容器内，跳过此步；DevContainer 用户同样已在容器里。否则在宿主机执行：

```bash
./scripts/run_dev_container.sh
```

脚本会自动复用已存在的 `sros-dev-env-main` 容器，没有则按 `.devcontainer/Dockerfile` 配置新建。镜像：`nexus3.sr:8060/sros/sros-dev-env:v5.69.0`。

下面所有命令均假设已在容器内。

### 编译

```bash
cmake --preset=amd64-debug && DISTCC_HOSTS="$(scripts/distcc_hosts.sh)" ninja -C build-amd64 -j$(scripts/build_jobs.sh)
```

并发度必须先走 `scripts/distcc_hosts.sh` 过滤不可达主机，再用 `scripts/build_jobs.sh` 计算可用槽位。写死 `-j$(nproc)`
会把 46 槽的 distcc 集群压到本机 8 核，实测全量冷编译 621s vs 296s。
ccache/distcc 的串接由 `CMakeLists.txt` 烧进 `build.ninja`，不依赖 shell 环境变量。

### ARM64 交叉编译

- 预设 `arm64-release`，Toradex SDK，编译器 `aarch64-tdx-linux-g++ 9.3`

### 测试

- 框架：C++ GoogleTest（`*_test.cpp`）、Python unittest（`*_test.py`）
- 编写异步或状态流转测试时，优先等待目标条件成立（轮询状态 / 消息 / 接口返回，单次短间隔，整体超时留足 CI 慢机余量），避免用 `sleep` / `usleep` / `std::this_thread::sleep_for` / `time.sleep` 这类固定延迟当同步手段；确需固定延迟时说明原因，并尽量只作为退避间隔
- 跑测试前必须先生成运行时环境：

  ```bash
  ROOT_DIR=$PWD ARCH=amd64 python3 scripts/generate_sros_running_context.py
  cd build-amd64 && ctest -LE "arm64|vda5050|vda5050_v2" --stop-on-failure --verbose
  ```

### 已知编译陷阱

- ARM64 EPSILON 宏冲突：rerun include 须在 perception 前
- 链接符号冲突：需 `--allow-multiple-definition`

## CI/CD

- 配置：`.gitlab-ci.yml`，阶段 `prepare → lint → build → test → pack → deploy → long_duration_test → release → trigger-integration-tests → notify → backport`
- 维护中的 release 分支由 `scripts/maintained_release_branches.json` 这一份快照承担事实源；`backport.py` / CI `auto_backport` job 都从这里读，禁止重新硬编码。`release.py::MAINTAINED_BRANCH_WINDOW`（默认 3）限定 ONES 对应的已存在维护分支数量，最终清单还会额外合入本次新 release。
- `release.py` cut 新 release 后刷新清单：从 ONES 标题含 `[待发布]` 的全部候选中，先过滤 origin 尚不存在的分支，再取最新 N 个，额外合入本次成功切出的 release，去重后按版本升序写入。提前规划但尚未建分支的 ONES 版本不占维护名额，也不与本次 release 比较大小。本次新 release 也可能入选维护窗口，必须保留其真实 ONES 来源；仅由 `extra_release_branch` 记录已确认的额外项。ONES 不可达时用旧快照兜底，只替换该显式标记且无 ONES 来源的旧额外项；旧快照缺标记或身份不明时保守保留，继承可用来源并过滤远端已不存在的分支。远端查询或写入失败时保留旧清单、继续升版，提交信息必须明确标记 `maintained release branches not refreshed`。
- 紧急人工修正：直接编辑 JSON 并 commit；commit message 需含 `manual edit maintained_release_branches` 便于追溯
- 自动 backport：带 `backport` 标签的 MR 合入 main 后，`.gitlab-ci.yml::auto_backport` 触发 `scripts/ci_auto_backport.py`，由 sros-bot（`GITLAB_ROBOT_TOKEN`，`api + write_repository` scope）建 cherry-pick MR，默认把 `assignee` / `reviewer` 都设为原 MR 作者；当原作者无法被设为 reviewer（离职 / 权限组不覆盖 / bot 权限不足）时会自动降级为只带 assignee 重试一次。冲突或异常时还会发飞书卡片 `<at>` @ 原作者；成功路径只在原 MR 上评论结果，不打扰群

## GitLab API 访问

查 CI 日志、操作 MR/Issue 等需求详见 [.agents/skills/gitlab-api/SKILL.md](./.agents/skills/gitlab-api/SKILL.md)。

## MR 与提交规范

- Multica 指派 issue 是 AI agent 的默认工作入口。完成本地修改和必要验证后，应直接 `git commit`、`git push` 并创建 MR，再把 MR 地址回填到对应 Multica issue 等待评审；不要额外等待“可以提 MR”的口头确认
- 非 Multica 指派、且用户没有主动要求提交的场景，仍禁止自动执行 `git commit` / `git push` / 新建分支操作
- 所有修复和实现必须从主干新建独立分支后提交 MR，禁止直接在 `main` 上提交
- MR 标题格式：`<TRACKING_ID> <type>(<scope>): <中文 subject>`。`TRACKING_ID` 支持 ONES `#150962` 或 Multica `EAS-22`；Multica 指派任务直接用 issue key，不必为了提 MR 额外新建重复 ONES
- 创建 MR 时默认选择合并后删除源分支和压缩合并；backport 类 MR 默认不指定评审人，除非用户明确要求
- 已发布 MR 后不要 `push -f`，只用普通 `git push` 追加提交，便于用户和评审人查看增量修改
- 不要为了解决 GitLab `need_rebase` 状态主动 merge/rebase 目标分支到源分支，避免无意义触发 CI；无冲突时由评审合并时执行 rebase without pipeline
- 评审意见必须继续在同一 MR 上追加提交和回复；只有用户或评审明确表示“评审通过”后，才能清理 worktree，并将对应 Multica/ONES 跟踪项流转到完成或测试验证
- AI 辅助代码必须在 commit message 和 MR 描述底部的 trailer 区域同时披露，格式为 `Assisted-by: <工具名>:<模型版本>`，例如 `Assisted-by: Codex:gpt-5.5`、`Assisted-by: Claude:claude-opus-4-7`
- AI 工具不能作为作者或签署主体；如需 `Signed-off-by`，必须由实际负责审查和提交的人填写

## 单位约定（易混淆）

| 数据结构 | 单位 |
|----------|------|
| `sros::core::Pose` (x, y, yaw) | m, rad |
| `sros::core::Velocity` / `SRCState` | mm/s, mrad/s |
| `setVelocity()` 入参 | mm/s, mrad/s（GzSimSrcSdk 内 ÷1000 发给 Gazebo） |
| `Pose.timestamp()` | μs (steady_clock)，**必须设置** |

## 数据库

- 路径：`/sros/db/main.db3`（SQLite + SQLiteCpp），硬编码
- 运行时路径 `/sros/` 在 `core/db/db.h:7` 硬编码，跑测试前必须通过 `scripts/generate_sros_running_context.py` 生成
- **func_config INSERT 必须提供 `id`（非空整数）和 `parent_key`（非空字符串）**
- 配置读取：`Settings::getInstance().getValue<T>(key, default)`
- 设备关联：`Settings::getDeviceItemByKey(func_key)` → func_config + `related_device_id` → dev_config
- **dev_config JSON 更新必须用 Python sqlite3**（命令行 sqlite3 破坏 JSON 双引号）

## 网络协议

- 定义：`cfg/sros-resources/protobuf/main.proto`（TCP，端口 5001，来自配置 `network.riot_server_port`）
- 分发：`modules/network/network_module.cpp`
- `CMD_SET_MANUAL_SPEED (0x10)`: 速度从 `param_int`(vx)/`param_int1`(w)/`param_int3`(vy) 读取，**不是** `pose`
- `CMD_SET_LOCATION_INITIAL_POSE`: 必须在 `CMD_START_LOCATION` **之前**单独发送
- `convertPose()` 对 pose ÷1000（mm→m, mrad→rad），与 `param_int` 互不影响
- 新命令务必查看 `network_module.cpp` handler 确认参数来源
- `SYS_STATE_TASK_NAV_NO_WAY (0x0B)`：导航失败后需发 `CMD_STOP_MOVEMENT (0x05)` 恢复到 IDLE
- `CMD_CANCEL_MOVEMENT_TASK (0x21)` 在 `default` 分支处理，转发到 `main_module.cpp:890`

## 对外交互与外部兼容

改动 sros（运控、导航、感知、动作等所有模块都是 sros 的一部分）前，先判断是否影响 sros 与**外部系统**的交互。内部重构只要对外的数值、行为、时机不变，外部就感知不到；一旦动到下面任一边界，必须评估外部兼容并在 MR 里说明。

sros 对外的边界（另一侧是独立的进程 / 上位系统 / 设备）：

- **网络 protobuf（TCP 5001，`main.proto` / `network_module.cpp`）**：命令下发 + 状态/故障/任务结果上报。对端 = 调度 FMS、Matrix HMI、`protocol_adapter/`（华为 LDM）、web。
- **VDA5050 v2/v3（`modules/vda5050_v*`）**：车队调度协议（订单 / 动作 / 状态）。
- **Modbus 寄存器（`modules/registers`）**：上位 PLC 读状态/故障码、写控制位。
- **SRC 底盘（`src/sdk` SrcSdk，串口 + SRTOS 寄存器）**：另一处理器上的 srtos 固件，plan_v 下发、反馈回填；仿真时换成 `GzSimSrcSdk`（gRPC→Gazebo）。

## 仿真系统

仿真专用参数及其目录必须按内部参数配置 `permission=0`，不得作为 `permission=99` 的普通参数展示到 Matrix；仍需运行时读取的参数保留 `is_valid=1`。优先使用既有 CSV 权限机制，不另加页面或接口过滤。新增或调整此类参数时，遵循 [配置参数权限说明](cfg/sros-resources/config/README.md#配置参数权限说明)。

仿真系统的架构、部署命令、gRPC 接口、验证脚本、已知陷阱等详见 [simulation/AGENTS.md](./simulation/AGENTS.md) 与 [simulation/CLAUDE.md](./simulation/CLAUDE.md)。

## 多 Agent 协作规则

本项目同时使用 Codex 和 Claude Code。请遵循以下规则：

1. **不要修改对方的配置文件**：
   - Codex 专属：`.codex/` 目录（sessions、memories）
   - Claude 专属：`.claude/` 目录（memory、commands、settings）
   - 共享：`.agents/skills/`

2. **分支管理**：同一分支同一时间只由一个 agent 工作，避免冲突

## 项目级 Skill

共享 skill 位于 `.agents/skills/`，Codex 和 Claude Code 均可使用：

| Skill | 用途 |
|-------|------|
| `sim-deploy` | 仿真环境部署（本地/远程） |
| `sim-test` | 仿真 pytest 测试执行 |
| `sim-stability` | 多远程主机批量运行仿真用例，验证稳定性 |
| `remote-build` | 远程编译 + 部署 |
| `gitlab-api` | GitLab REST API：CI 日志、MR/Issue/Pipeline |
| `sros-config-parameter` | SROS 功能参数新增、评审、单位/车型/目录与升级验证 |
| `rk3399-flash` | Ubuntu 下给 RK3399 国产控制器（VC400-rk3399/VC500）刷 update.img、串口检查 eMMC |

## 语言

始终使用简体中文回复。
