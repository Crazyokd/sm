# AGENTS.md

本文件只记录 AI / 协作者改代码时必须遵守的规则。仓库入口、运行命令、发布流程和包内调试手册以 [README.md](./README.md) 及各子目录 README 为准。

回复统一使用简体中文。

本仓库承载 EasyGo 全部代码：后端（ROS 2 主仓，目标分支 `main`）与前端（`src/easygo-fe`，独立 Git 子仓，目标分支 `dev`）。默认在用户当前工作区按请求改动；如需隔离 worktree，用户会明确说明。

## 文档分工

- `README.md`：面向使用者，放仓库入口、启动、部署、发布、常用命令和 TODO。
- `src/easygo_bringup/README.md`：真机 bringup、建图、定位、导航、遥控等包内调试细节。
- `src/easygo_sim_bringup/README.md`：源码仿真 bringup、构建、自测细节。
- `docker/*/README.md`：镜像边界、构建、发布和运行时细节。
- `docs/testing/test-development-guide.md`：所有测试脚本、fixture、协议 client、Page Object 和测试 helper 的统一开发规范。
- `docs/testing/assert_rules.md`：断言有效性、失败处理和 ONES 追溯的细则。
- `AGENTS.md`：只放协作纪律、改代码前的核实动作、不要做什么。

同一事实不要在 README 和 AGENTS 两边各维护一份。需要引用操作命令时，优先指向对应 README。

## 改代码前

- 先读相关代码、README 和 spec，再下手改。
- 不凭目录名、镜像名、脚本名猜当前行为；涉及 image 内容、工具路径、运行时文件时，先实际验证。
- 工作区可能已有他人改动；不要回滚、覆盖或格式化无关文件。
- 用户未要求时，不自动 `git commit`、`git push`、新建分支或 rebase。

## 子仓归属

`src/` 下混合存在主仓内包和独立 git 子仓。修改 `src/<pkg>` 前必须先确认归属：

```bash
git -C src/<pkg> rev-parse --show-toplevel
```

- 输出为主仓根目录：包随主仓提交。
- 输出为 `src/<pkg>`：这是独立仓，需要在子仓单独开分支 / MR；主仓不带子仓代码改动。
- `src/easygo-fe` 是前端独立仓（远端 `standard-fe/easygo`，目标分支 `dev`），前端变更在该子仓单独开分支 / MR；构建 / lint / MR 细节参考 `src/easygo-fe/.agents/skills/easygo-frontend-merge/`。

不要只凭路径或包名判断。

## 设计文档

跨仓、镜像、CI、运行方式等改动前先读 `docs/specs/` 下相关 spec。

新建或更新 spec 时：

- 文件名使用 `YYYY-MM-DD-<主题>.md`。
- 第一行用 blockquote 标注状态：`> ✅ 已实现`、`> 🚧 部分完成`、`> 📋 待实现`。
- 已实现 spec 不删除，它们保留设计决策背景。
- 已修复的开放问题项应删除并重新编号，不要保留 `~~划掉~~ ✅`。

## 编程原则

- 遵循 SOLID、KISS、DRY、YAGNI，只实现当前明确需要的功能。
- 修复回归优先写最小复现用例。仿真回归优先走 `easygo-sim` 或 launch 测试；ROS 节点尽量用 `launch_testing`。
- 注释语言与所在代码保持一致。
- PoC 阶段不要引入“过渡 / 双轨 / 切回旧方案”开关。遇到 apt-only、amd64-only、ABI 基线等硬约束冲突时停止并向用户说明。

## 测试脚本开发规范

- 新增或修改 `test/`、`src/**/test/`、`deploy-test/` 下的测试脚本、fixture、Page Object、协议 client 或 helper 前，必须先阅读 [测试脚本开发规范](docs/testing/test-development-guide.md) 和 [断言规则](docs/testing/assert_rules.md)。
- 测试必须以需求、ONES、Issue 或协议条款为依据，形成“前置 -> 动作 -> 状态迁移 -> 最终结果 -> 恢复”的可失败闭环；不能用非空、无异常、页面可见、消息已发送或固定等待代替业务断言。
- UI 测试遵循稳定的用户语义选择器、Playwright Web-first 等待、Page Object/fixture 分层和精确资源所有权；所有测试记录真实来源与覆盖边界，具体要求以测试开发规范为准。Modbus、VDA5050 和仿真测试还必须区分业务驱动与独立物理观测，并核对协议字段和物理副作用。
- 失败路径、清理和全局状态恢复不能吞掉原始失败；涉及机器人运动或配置写入时必须具备安全停止、取消、回读和恢复验证。

## 容器纪律

- 本机不安装 ROS / colcon / cargo 等工具链。编译、测试、格式化、发布相关命令默认在开发容器内执行。
- 先用 `uname -n` 自检；主机名不是 `easygo-dev-main` 时，按 README 的“开发容器”入口进入容器。
- 容器内缺工具或依赖时，先报告用户；不要直接 `apt install` / `pip install`。依赖变更应进入 base image 或相关 Dockerfile。

## 仿真纪律

### 全量仿真：单车 / 多车必须兼容（强约束）

**调整全量仿真时，必须同时考虑单车全量仿真与多车全量仿真的兼容性。禁止只验证一种模式、导致另一种模式回归。**

- **单车全量仿真**：`easygo-sim start` → `sim_easygo.launch.py`（本进程启 Gazebo + 完整单车栈）。
- **多车全量仿真**：`easygo-sim multi` → 1 × `sim_multi_gazebo.launch.py` + N × `sim_easygo.launch.py start_gazebo:=false`（共享 Gazebo，每台车独立完整单车栈）。细节见 `src/easygo_sim_bringup/README.md`「多车（共享 Gazebo 世界）」。
- 两者共用 launch、脚本、observe/webgz、端口与 topic 约定。**改共享路径时，必须确认两种入口都仍可用**；不能假设「只在单车里测过就没问题」或「多车只是 compose 包装、与 launch 无关」。
- 交付前至少满足其一：**两种模式都跑过**；或 **用 launch 测试 / 代码审查明确证明** 变更只影响其中一种且不会波及另一种（例如改动被 `start_gazebo:=false` 或 `robot_count` 严格隔离）。
- 常见互踩点（改到相关代码时必须双线核对）：全局 Gazebo / `pkill`、GZ topic 命名（全局 vs 命名空间）、`ROS_DOMAIN_ID` 与 DDS 域、`/clock` 与 TF 图、host 网络端口偏移、observe / 驾驶台 API 的 `robot_name`、多车共用 world 时的 GUI / `<camera_pose>`。

- AI / CI 自测不要启动 Gazebo GUI；需要 launch 仿真时显式使用 headless 参数，具体命令看 `src/easygo_sim_bringup/README.md`。
- 仿真默认应限制 DDS / Gazebo 多播在本机。跨机器调试需要广播时，由用户显式设置环境变量。
- 跑多实例时先查端口占用，避免抢占 `11345`、`9090`、`1883`、`8080`、`18088` 等端口。

### 仿真自测推荐做法

- 起 sim 用 `setsid bash -c '...' > log 2>&1 < /dev/null &` 脱离会话；拆栈用 `bash src/easygo_sim_bringup/scripts/sim-kill.sh`（分级优雅停）。
- sim 起一次、多次复用，只在重编译后重起；验证只 grep 需要的日志行，不要把整坨 sim 日志读进来。
- 优先用单测 / launch_testing / 手动 `ros2 param set` + grep 节点 INFO 日志验证，能不起全量 Gazebo 就不起。`ros2 param get` 读的是声明参数、看不到节点内部成员改动，内部生效看节点日志。
- nav2 由 easygo-app orchestrator 统一启动（不再等定位），costmap 内部非阻塞等待 TF。仿真如需手动放行可 `ros2 service call .../manage_nodes nav2_msgs/srv/ManageLifecycleNodes "{command: 0}"`。

### Organism Agent 修复验证（强约束）

- Organism Agent 修复 EasyGo 问题（issue / bug）时，**必须用全量仿真 MCP（`easygo-full-sim`）做端到端验证**：`sim_start` / `sim_add_robot` 起栈，`sim_status` / `sim_logs` 确认就绪后再验证修复点；禁止只编译通过或只看日志就交付。
- 涉及前端交互（WebApp 点击、下发巡逻 / 配送任务等）的验证，必须用 Playwright（如 `sim_ui_run_task`）驱动操作，并**录制操作过程视频**作为交付证据，不接受无录屏的「已点击验证」结论。
- 全量仿真 MCP 多车验证**最多同时使用 10 台车**（含 `sim_start` 的 `robot_count` 与 `sim_add_robot` 累加），超出先 `sim_remove_robot` 回收再加车。

## 真机纪律

- EVT 车 SSH 入口见 `docs/EVT2-*_robot_state.md`（用户 `std`；优先 `ssh EVT2-x` 或 `std@<IP>`）。首次连机用 `ssh-copy-id std@<IP>` 录入公钥，**不要在仓库或脚本里写密码**。
- 真机首次部署 / 升级、运维命令以 README 和 `docker/real/README.md` 为准。
- 排查硬件问题先查 [docs/hardware_debugging/](docs/hardware_debugging/README.md)，优先确认历史边界和验证矩阵；没有实测证据不要把问题归因到硬件瓶颈。
- 真机 ROS 2 终端调试使用 `/sros/ota/launch shell`，不要直接写成 `docker exec -it -u std easygo bash`。
- root shell 只用于系统、权限、`dpkg`、`supervisorctl` 排查。
- **不同车之间禁止拷贝数据库**，换控制器除外。序列号、标定数据跟车走。
- Orbbec DCW2 udev 规则属于宿主机，不属于容器。
- RViz2 不随整车 bringup 自动启动；真机 RViz 调试从本地脚本入口走。

## CI/CD

- CI 入口是 [.gitlab-ci.yml](./.gitlab-ci.yml)。不要凭记忆写 job、镜像、tag 规则或 component 版本，改 CI 前先读文件。
- 引用 `SROS/infra/ci-templates` component 时，先用 GitLab API 核实已发布 tag，不要照抄 examples 中未发布版本。
- 应用 ABI 锚定容器运行环境；交叉编译 sysroot 和 glibc 基线不要为了兼容某台宿主机私自降级。
- 正式发布物不要推到 `docker-registry.nowhere.sr`。

## 发布纪律

三条发布线各自的网页入口见 [README 的「发布」章](./README.md#发布)。这里只放纪律。

- **发布动作走网页入口，不要用 API 或裸命令建 tag。** 用户没有明确要求时，agent 不得用 GitLab
  API 创建 `real-v*` / `v*` tag，也不得直接 play 发布 job。发布是人的决定，入口就是流水线上那
  一次点击。用户明确点名要发某个版本时照做，并回报实际做了什么。
- **版本号不要临时拼。** 三条线的版本号由发布脚本自己算（main 抬 Minor、release 分支抬 Patch，
  见 README「版本号规则」）。手填版本号正是 `0.3.145~1` 这种四段 tag 的来源——它在 Debian 里
  是预发布语义，与「修订」意图相反。
- **仓库里改对了不等于生效。** 凡是涉及远端脚本、SSH forced command、镜像、参数中心的改动，
  合并后必须确认目标机器上真的换了；只看到 MR 合并不算完成。2026-09 连续踩过三次：
  `ota-promote-prod` 在仓库里躺了两周没装到 ota-hub、板子门禁的清理命令从第一天起就没生效过、
  ota-client 的修复合进 main 却没发版所以车上仍是旧行为。
- **没跑过的路径不算能用的路径。** 新增或改动发布链路后，交付前要么真跑一次，要么明确说明哪
  一段还没验证过、首跑会发生在什么时候。
- **没人看的流水线不是门禁。** 发布链路上的失败必须红在有人会看到的地方。2026-09-11~13 板子
  门禁连红三晚无人处理，就是因为它红在一条 bot 创建、无人订阅的 tag 流水线上，而 daily release
  自己报的是 success。

## MR 与提交

- 新建 MR 时标题使用中文，简洁描述本次变更。
- 后端 MR 目标分支 `main`；前端 MR 目标分支 `dev`。跨前后端改动，两边各提一个 MR。
- 交付 MR 时必须附 MR 的 web 链接，方便评审直接打开：
  - 后端：`http://git.standard-robots.com/SROS/platforms/easygo/-/merge_requests/<iid>`
  - 前端：`http://git.standard-robots.com/standard-fe/easygo/-/merge_requests/<iid>`
- 已发布 MR 不要 `push -f`，追加普通提交即可。
- 不要为了消 GitLab `need_rebase` 主动 merge / rebase 目标分支到源分支；无冲突时让评审合并时处理。
- `发布 real`、`升级真机`、`修 CI`、`现场救场` 都不等于授权把未合并 MR 或 feature 分支并入 `main`。这些任务只能消费已经在 `main` 上的代码；如果发现疑似修复在未合并分支里，停止并报告，或按用户明确要求另提 MR。
- 未合并 MR 一律视为未发布代码，即使它看起来是根因修复、CI 曾经成功，agent 也不能自动 cherry-pick、merge、rebase 或直接 push 到 `main`。
- 现场测试失败、结论为 partial、MR 仍 opened、或用户曾要求“测试完失败先不管”的分支，视为禁止自动发布来源；除非用户明确点名要把具体提交直接进 `main`，否则只能用于只读分析或重新提 MR。
- 自动发布 / 定时发布 agent 只负责 release 流程、CI 状态跟踪、OTA 发布和真机验证。发布过程中遇到代码 blocker 时，应停下来给出证据和建议；不能在 release checkout 里临时 cherry-pick 未合并业务代码来继续发布。
- AI 协作需要在 commit message 或 MR 描述 trailer 区域披露，例如 `Assisted-by: Codex:gpt-5.5`。
- AI 工具不能作为 author 或 `Signed-off-by` 主体。

## GitLab API

环境变量 `GITLAB_PRIVATE_TOKEN` 已设。查 MR、pipeline、job trace、仓库文件时优先用 GitLab REST API，避免网页抓取。

常用 project id：

- easygo 主仓：`1147`
- ci-templates：`1164`

常用 endpoint：

```text
GET /api/v4/projects/:id/merge_requests/:iid
GET /api/v4/projects/:id/merge_requests/:iid/changes
GET /api/v4/projects/:id/pipelines?ref=<tag>&order_by=id
GET /api/v4/projects/:id/pipelines/:pid/jobs
GET /api/v4/projects/:id/repository/tags?search=<prefix>
GET /api/v4/projects/:id/repository/files/<urlencoded>/raw
```

## 多 Agent 协作

本仓可能被 Claude Code 与 Codex 同时使用：

1. 不要修改对方专属目录：
   - Claude：[.claude/](.claude/)
   - Codex：`.codex/`
2. 同一分支同一时间只由一个 agent 工作。
