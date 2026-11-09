# AGENTS.md

仓库规则。子目录 `AGENTS.md` 可覆盖其范围。

## 边界

- **本仓**：机器人仿真编排框架 + 当前内置的 Gazebo 世界 Port 与 EasyGo 车端 Port。框架不解释任何一种仿真器或车端；Gazebo server、world SDF、机器人模型、spawn、GZ `clock` 是 Gazebo Port 的实现细节。框架层不起 easygo-app / SLAM / Nav2 / WebApp / 驾驶台 / RViz。
- **新增实现**：新增仿真实现或车端实现 = 新增一个 `internal/ports/**` 包 + `cmd/simenv` 组合根注册表加一行。框架（`internal/contract`、`internal/orchestrator`、`internal/assets`、`internal/cli`）一行不改。`internal/ports/{world,vehicle}/null` 是零容器的状态文件对：无 Docker 开发模式，也是这条规则的架构自检。
- **种类路由**：实现种类（kind）由 bundle `worlds`/`vehicles` 唯一键推导；裸世界必须显式给 `--world-kind` / `--vehicle-kind`。运行中的会话名单在 `$XDG_CACHE_HOME/simenv/session.json`：每条记名字、世界种类、车端种类、资源槽。`start` 不写 `--session` 就生成一个名字（如 `brisk-otter`），写了就用指定的。`simenv status` 列出全部，`simenv status <name>` 展开这一次；`simenv stop` 停最近启动的那一个，`simenv stop <name>` 停指定的。点名的 pose/reset/call 把会话名放在命令后面。未选中的实现不被构造，不逐实现盲探。Port 的发现与停止不得依赖 start-time 参数（发现路径拿不到它们）。
- **程序**：现有 EasyGo `runtime-sim` + `sim_easygo.launch.py start_gazebo:=false`。由 `--app-image` 传入。程序不读本仓源码，不起 Gazebo server。
- 两边不共享镜像、不共享 launch、不共享版本号。环境镜像不以 `runtime-sim` 为 `FROM`。运行时不挂 EasyGo 工作区。
- **会话契约**：`Session` 只含 `World`、`Robots`；Robot 只含 `Index`、`Name`、`Pose`（世界系，m / rad）。`Partition`、`TopicPrefix` 是 Gazebo Port 的接线约定，经 `Attachment` 交给车端，不进契约。
- **Gazebo Port 约定**：实体与 GZ topic 前缀 `robotN`（N 从 1）；`GZ_IP=127.0.0.1`；`GZ_PARTITION=easygo_sim`。只约束 `internal/ports/world/gazebo`。
- **EasyGo Port 约定**：程序 `ROS_DOMAIN_ID=11+(N-1)`；环境 `ROS_DOMAIN_ID=1`；HTTP `frontend=18088+100N`、`api=18089+100N`。N=1 的 API 是 `:18189`，不是旧单车 `:18089`。只约束 `internal/ports/vehicle/easygo`，不得进入 `internal/contract` 的类型。
- **入口**：`cmd/simenv` 宿主编排与组合根（`start` / `status` / `stop`，以及运行中会话的 `pose` / `reset` / `call`）；`cmd/simenv-world-gazebo` 环境容器内世界进程。Docker Engine API 起停容器，不拼 `docker run`。控制面不进 `internal/contract`。编排器只路由：`pose` / `reset` 的位姿（世界系）交给 `WorldPort`，`call --robot` 把标准输入原样交给该车的 `VehiclePort.Forward`，再把读到的字节写回标准输出。本仓不解析这段字节，不维护车端动词表。gz 只活在 `internal/ports/world/gazebo` 里。
- 禁止 `*.sh` 业务入口、Python 编排、把 TypeScript 当本仓实现语言。不包装 `full_sim.sh` / `easygo-sim` / `sim_multi_gazebo.launch.py`。
- 不嵌进 TP 或 EasyGo CI。操作员 skill：起停环境 `.agents/skills/simenv-run/SKILL.md`。
- EasyGo / Gazebo 概念只作为各自 Port 包的实现依赖，不得进入会话契约、编排框架或资产装载。

## 改动

- 契约 `internal/contract/`，编排 `internal/orchestrator/`，资产装载 `internal/assets/`，Port 实现 `internal/ports/**`，入口 `cmd/`，环境镜像 `deploy/`。设计合同 `docs/superpowers/specs/`。
- 框架包（`internal/contract`、`internal/orchestrator`、`internal/assets`、`internal/cli`）禁止 import `internal/ports/**`，源码禁止出现 gazebo / gz / easygo / docker / ros 字样；CI `rg` 断言。依赖方向只有 `ports/** → orchestrator → contract`。
- 禁止改 `.codegraph/`。不提交 `docs/superpowers/plans/`（本机施工单；合同在 `docs/superpowers/specs/`）。
- Go、带类型配置进 struct。非法字段直接失败，不设默认值迁就脏输入。
- 不做无关重构、格式化或搬文件。只暂存本任务文件，保留用户已有改动。
- 未发版：禁止兼容和兜底。不做旧字段映射、双路径、legacy 别名，也不用默认值或吞错迁就脏数据。非法输入直接失败；碰到兼容路径就删。
- 行为变化补 `go test`。本仓测试只测会话契约算术、编排状态机、Gazebo 世界进程对 clock/实体的等待、各 Port 的 Attachment / Options 解析与坐标换算，以及车端 Port 的参数算术。
- worlds / 机器人模型来自 pinned EasyGo 修订，记录在 `deploy/assets/EASYGO_WORLD_SOURCE_SHA`。换 SHA 必须同步镜像 LABEL 与该文件。

## 验证

- 文档：`git diff --check`；命令 / 端口 / 路径与源码一致。
- 代码：`go test ./...`；改入口或镜像时 `go build -o simenv ./cmd/simenv` 与 `go build -o simenv-world-gazebo ./cmd/simenv-world-gazebo`，需要环境镜像时 `docker build -f deploy/Dockerfile -t simenv-world-gazebo:local .`。
- 框架守卫：`! rg -in "gazebo|gz|easygo|docker|ros" internal/contract internal/orchestrator internal/assets internal/cli`。
- N=1 真跑门禁（环境具备时）：`simenv start` 退出 0；环境有 GZ `clock` 与实体 `robot1`、进程无 easygo-app；程序 `GET http://127.0.0.1:18189/api/v6/vehicle/status` 可达；`status` 分开报告；`stop` 停干净。本机没 Gazebo / `runtime-sim` 时写明未验证，禁止把「没跑多车」写成环境缺陷。
- 无 Docker 冒烟：`./simenv start --world nullworld --robots 2`（null 对零参数，镜像 flag 只对 gazebo/easygo 生效）→ `status` / `pose` / `reset` / `call` / `stop` 全链路应可用。

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
