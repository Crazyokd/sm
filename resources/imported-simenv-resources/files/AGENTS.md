# AGENTS.md

仓库规则。子目录 `AGENTS.md` 可覆盖其范围。

## 边界

- **本仓**：Gazebo server、world SDF、机器人模型、spawn、GZ `clock`、宿主编排。不解释车端，不起 easygo-app / SLAM / Nav2 / WebApp / 驾驶台 / RViz。
- **程序**：现有 EasyGo `runtime-sim` + `sim_easygo.launch.py start_gazebo:=false`。由 `--app-image` 传入。程序不读本仓源码，不起 Gazebo server。
- 两边不共享镜像、不共享 launch、不共享版本号。环境镜像不以 `runtime-sim` 为 `FROM`。运行时不挂 EasyGo 工作区。
- **会话契约**：`Session` 只描述 World、Partition、Robot 清单；Robot 只含 `Index`、`Name`、`TopicPrefix`、`Pose`。实体与 GZ topic 前缀默认 `robotN`（N 从 1）；`GZ_IP=127.0.0.1`；`GZ_PARTITION=easygo_sim`。
- **EasyGoVehicle 实现约定**：程序 `ROS_DOMAIN_ID=11+(N-1)`；环境若需要 ROS 则 `ROS_DOMAIN_ID=1`；HTTP `frontend=18088+100N`、`api=18089+100N`。N=1 的 API 是 `:18189`，不是旧单车 `:18089`。这些不得进入 `internal/contract` 的类型。
- **入口**：`cmd/simenv` 宿主编排（`start` / `status` / `stop`）；`cmd/simenv-world` 环境容器内世界进程。Docker Engine API 起停容器，不拼 `docker run`。
- 禁止 `*.sh` 业务入口、Python 编排、把 TypeScript 当本仓实现语言。不包装 `full_sim.sh` / `easygo-sim` / `sim_multi_gazebo.launch.py`。
- 不嵌进 TP 或 EasyGo CI。操作员 skill：起停环境 `.agents/skills/simenv-run/SKILL.md`。
- EasyGo 概念只作为程序镜像与 `VehiclePort` 边缘实现依赖，不得进入会话契约或环境实现。

## 改动

- 契约 `internal/contract/`，编排 `internal/orchestrator/`，世界进程 `internal/world/`，入口 `cmd/`，环境镜像 `deploy/`。设计合同 `docs/superpowers/specs/`。
- 禁止改 `.codegraph/`。不提交 `docs/superpowers/plans/`（本机施工单；合同在 `docs/superpowers/specs/`）。
- Go、带类型配置进 struct。非法字段直接失败，不设默认值迁就脏输入。
- 不做无关重构、格式化或搬文件。只暂存本任务文件，保留用户已有改动。
- 未发版：禁止兼容和兜底。不做旧字段映射、双路径、legacy 别名，也不用默认值或吞错迁就脏数据。非法输入直接失败；碰到兼容路径就删。
- 行为变化补 `go test`。本仓测试只测会话契约算术、编排状态机、世界进程对 clock/实体的等待，以及 `VehiclePort` 边缘实现自身的参数算术。
- worlds / 机器人模型来自 pinned EasyGo 修订，记录在 `deploy/assets/EASYGO_WORLD_SOURCE_SHA`。换 SHA 必须同步镜像 LABEL 与该文件。

## 验证

- 文档：`git diff --check`；命令 / 端口 / 路径与源码一致。
- 代码：`go test ./...`；改入口或镜像时 `go build -o simenv ./cmd/simenv`，需要环境镜像时 `docker build -f deploy/Dockerfile -t simenv-world:local .`。
- N=1 真跑门禁（环境具备时）：`simenv start` 退出 0；环境有 GZ `clock` 与实体 `robot1`、进程无 easygo-app；程序 `GET http://127.0.0.1:18189/api/v6/vehicle/status` 可达；`status` 分开报告；`stop` 停干净。本机没 Gazebo / `runtime-sim` 时写明未验证，禁止把「没跑多车」写成环境缺陷。

### 按项目 skill 做事时：必须交「skill 执行记录」

项目 skill（`.agents/skills/*/SKILL.md`）不是照抄一遍就完事：**它同时是过程记录的对象**。凡是按某个项目 skill 完成的任务，回复里必须带一小节「skill 执行记录」，让用户能判断这个 skill 该不该改——**改不改由用户决定，agent 不擅自重写**。

记录格式（3~6 行，不写流水账；没偏差就写「无」）：

```
skill=<name>  目标=<这次要干的事>
偏差：<skill 没写的步骤 / 命令失败后换法子 / 同一处重复两次以上 / 耗时明显超预期>
过时：<skill 里与现状不符的路径、命令、flag、常量>
冗余：<这次完全没用到、也想不起它救过我的段落>
建议：<每行给出改法，标明归属：世界层 / 接线层 / app 侧 / skill 表述>
```

**必须上报的触发条件**（命中任一即写）：skill 里没有的步骤；命令失败后不得不换做法；同一处重试/重复两次以上；实际耗时或复杂度明显超出 skill 的预期；skill 引用的路径/命令/flag 已不存在或行为变了。

**不报**：纯参数差异（world 名、镜像 tag、机器人台数）。

**允许就地改的**：机械且无争议的修正（引用的路径搬了、命令名错了、漏了必做的一步），改在**同一轮**里做完（对话会被压缩，"以后记得改"必然丢），并在「skill 执行记录」里点名"已就地改"。

**语义改动**（删步骤、改流程、判定某段冗余、换判据）**一律只提建议，等用户拍板**。

## Git

- Conventional Commits。一提交一逻辑。不提交密钥、生成物、运行时数据或无关文件。
- 默认在 `main` 上直接开发、提交并 push `origin main`。
- 开发前 / 推送前：`git fetch origin`，`main` 先 fast-forward 到 `origin/main`，再提交已验证改动。
- 「提交」：在当前 `main` 落 commit。
- 「提交&push」：提交后 push `origin main`。
- 「提交MR」：仅在用户明确要求走 MR 时使用；从功能分支 rebase `origin/main` 后 push，并 `glab mr create`（已有则更新）开向 `main`。开完或更新后跟到 CI 绿；红了修、再推，直到 pipeline success。未绿不算就绪。
- 一份 MR 可含多次提交。同一件事堆同一分支，不要为合入压成一颗，也不要 squash 掉逻辑提交。
- `main` 线性：只允许 fast-forward 或普通线性提交。禁止 `--no-ff`、`merge:`、在 `main` 上 pull 出 merge。禁止 force-push `main`。
- 未要求不删功能分支。
