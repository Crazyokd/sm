# AGENTS.md

仓库规则。子目录 `AGENTS.md` 可覆盖其范围。

## 边界

- **Server**：注册 Adapter、暴露 Capability、校验/持久化 Case、Run 入队。不解释 actor，不碰真实资源。
- **Worker**：进程内固定线程，一次一个 Run，自己从内存队列领取。
- **Adapter**：声明 typed leaf Capability；每个 Run 独立 Session，由它解释 actor、资源和执行语义。
- **Case**：`@tp/cases` 编树与安装器。不属于任何一个 Adapter。叶子必须来自已加载 Adapter 的目录。
- 三方只依赖 `@tp/contract`，经 Registry / Run Queue / Contract 解耦。日志 `@tp/log`。记账 `@tp/metrics`：主机记 Case/Run/队列/Worker/HTTP，读数 `GET /api/metrics`（`collect()` 快照 DTO）；Adapter 在 isolate 记 session/tick。Server 源码不得出现具体 Adapter，只靠 `adapterModules` / `TP_ADAPTER_MODULES` 加载。
- **MCP**：独立 stdio 进程，只依赖 `@tp/contract`，HTTP 调已启动的 Server。不嵌进 Server，不解释 actor。
- EasyGo 概念不得进入平台核心。平台不调用 deploy，不校验版本。
- 操作员 skill：跑已有 Case `.agents/skills/tp-run-case/SKILL.md`；改编排 `.agents/skills/tp-edit-case/SKILL.md`；EasyGo 热更新后再跑 `.agents/skills/easygo-tp-hotupdate-verify/SKILL.md`。
- 引擎对齐 BT.CPP v4（tick/halt、节点状态、控制节点与装饰器）。允许 `ReactiveSequence` / `ReactiveFallback`，以及 `Repeat` / `RetryUntilSuccessful` 的 `-1`。
- Run 可以一直 `RUNNING`；只有取消才 halt 整树并回收 Worker / Session。`Fallback` 是 BT 节点，不是兼容层。

## 改动

- 契约 `packages/contract/`，日志 `packages/log/`，指标 `packages/metrics/`，后端 `packages/server/`，前端 `packages/web/`，MCP `packages/mcp/`，Adapter `packages/adapters/<name>/`，Case 编排 `packages/cases/`（不进 Adapter），跨包测试 `test/`。
- 禁止改 `node_modules/`、`dist/`、`data/`、`.codegraph/`。不提交 `docs/superpowers/plans/`（本机施工单；合同在 `docs/superpowers/specs/`）。
- TypeScript strict、ESM、显式 `.ts` 导入。API 输入 `zod`，领域复用 `@tp/contract`。
- 不做无关重构、格式化或搬文件。只暂存本任务文件，保留用户已有改动。
- 架构 / 数据模型 / Worker / Adapter / UI 变化时，同步 `packages/web/public/architecture-atlas/index.html` 与运行时图源 `docs/archify/tp-runtime.architecture.json`（改源后 deliver 到 `packages/web/public/archify/index.html`）。界面身份以 `DESIGN.md` 为准，实现在 `packages/web/src/styles.css`。行为变化补 `node:test`。
- 未发版：禁止兼容和兜底。不做旧字段映射、双路径、migrate-on-read、legacy 别名，也不用默认值或吞错迁就脏数据。非法输入直接失败；碰到兼容路径就删。

## 验证

- 文档：`git diff --check`；命令 / 端口 / 路径与源码一致。
- 代码：`npm run format:check`、`lint`、`typecheck` 加相关测试；跨层或提交前 `npm test`。CI 在 `main` 与 MR 上用 `nexus3.sr/node:24` 跑这四道闸；MR 的 Prettier 只检查相对目标分支的改动文件。
- 环境跑不了全量时，跑最接近的检查并写明未验证项。

## Git

- Conventional Commits。一提交一逻辑。不提交密钥、生成物、运行时数据或无关文件。
- 禁止直推 `main` / `origin/main`。进入 `main` 必须走 GitLab MR。禁止在本地 `checkout main` 后提交再推，禁止 `git push origin HEAD:main`。
- 开发前 / 推送前：`git fetch origin`，功能分支 rebase `origin/main`。rebase 改写了已推送功能分支时，只对该分支 `--force-with-lease`。
- 「提交」：只在功能分支落 commit，不推 `main`。
- 「提交&push」：rebase 后只 push 功能分支。
- 「提交MR」：rebase 后 push 功能分支，`glab mr create`（已有则更新）开向 `main`。不要本地合 `main`。开完或更新后跟到 CI 绿；红了修、再推，直到 pipeline success。未绿不算就绪。
- 一份 MR 可含多次提交。同一件事堆同一分支，不要为合入压成一颗，也不要 squash 掉逻辑提交。
- `main` 线性：MR 只允许快进。禁止 `--no-ff`、`merge:`、在 `main` 上 pull 出 merge。禁止 force-push `main`。
- 未要求不删功能分支。
