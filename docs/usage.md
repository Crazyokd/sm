# 使用 SM

SM 在每台 Linux 机器上本地运行，管理同一份已审核资源。Skill 是客户端无关的单一内容；Hook、规则、Agent 定义和 MCP 配置按客户端原生格式适配。

## 初始化

构建需要 Rust 1.88+（Cargo）、C 编译器／链接器、Git 和可用的 systemd 用户会话。本程序的运行、记录与服务不依赖 Python；前端产物已随仓库提供。当前清单通过 Skills CLI 恢复上游 Skill，需要 Node.js 20+ 与 npm/npx。资源自带的业务脚本继续使用各自原有运行时。复用本机 Codex／Kimi 登录、Git 提交身份、GitLab 认证。提交 MR 需要已认证的 `glab`，夜间分析需要 `codex` 或 `kimi`；缺项可在初始化结果中查看。

```sh
git clone http://git.standard-robots.com/yicao/sm
cd sm
./sm init
```

初始化先通过锁定的 Cargo 依赖构建 `target/release/sm`，再生成当前用户的 SM 配置、Web 服务和每日 02:00 的 timer。重复执行用于补齐同名服务。打开 <http://127.0.0.1:8765>。

Web 每次启动会进行首次同步。夜间任务依次同步已合并的自建资源、拉取上游、应用受管文件、消化记录、分析自建资源并尝试提交 MR。机器错过计划时间后，由 systemd 的持久 timer 补执行；需要运行中的用户会话。配置中的工具路径来自初始化时的 PATH，修改工具安装位置后重新初始化。

本项目默认维护分支为 `main`。同步读取远端 `origin/main`，本地未提交或未合并的文件不会直接生效。首次在新远端使用前，应先提交并审核仓库内容。

## 本地状态

只支持当前 Rust 格式，不读取旧版所有权编码，也不保持旧提案编号和输出摘要算法。旧安装状态不支持直接复用或跨实现回退；程序不会自动清空数据库或覆盖无法识别的受管内容，遇到异常会报错。已有安装应先保留原状态及待审提案，人工解除旧投放后再初始化当前格式，不能仅删除所有权数据库后覆盖旧文件。

## 日常管理

1. 在“资源目录”阅读内容，按类型、来源、作用域筛选。Skill 显示“通用 Skill”，无需选择客户端。
2. 使用“新建资源提案”或详情的“发起修改”，填写内容、原因和用户／项目归属。修改资源名称会产生重命名提案，配套文件一并保留。新项目同时登记约定目录，如 `~/repo/app`。
3. 提交后打开 GitLab MR，由人工检查差异、证据和未验证项。网络或认证失败时保留本地提案，并在页面显示原因。
4. 审核合并后，下一次同步才更新生效目录。“申请移除纳管”也遵循这条流程。

“作用域与项目”汇总归属。“使用与改进”展示可见记录、改进发现和关联 MR。“运行状态”展示同步、分析和采集状态。文件已应用与客户端已加载是不同结果；SM 不会强制中断现有会话。

各台机器必须采用相同的项目命名和目录约定，并事先存在声明的项目目录。`~/` 根据本机用户目录展开；不提供每台机器的个人项目映射。

## 两种来源

**自建资源**保存在 SM 的 `resources/<资源名>/`。用户修改、AI 内容改进、归属调整和淘汰都走 MR。应用时只修改 SM 已登记拥有的内容；无关配置、凭据和本地文件保留。受管文件若被本地手改，会报告冲突并保留原文件，需要先人工处理冲突。

**上游资源**登记来源和内容目录。每次同步直接跟随上游当前内容，不为普通上游更新创建 SM MR，也不进入本地优化分析。来源和归属登记的修改仍须审核。除 Git 来源，清单也可登记 `upstream.argv` 生成器；它在已批准的暂存目录运行，输出目录作为最后一个参数，不应包含凭据。

## 采集与改进

日常 Hook 只追加本地记录，不调用模型；完整工具输出替换成摘要哈希。能匹配受管内容路径时记录资源关联，其他事件标记为部分可见。没有记录不等于未使用，读取 Skill 文件也不能证明每一步都被遵守。当前不采集模型内部推理。

夜间分析仅处理自建资源，优先复用 Codex，否则使用 Kimi。分析角色禁用业务工具和子 Agent，避免执行被分析资源。它把自建内容和脱敏后的可见记录交给该 CLI 配置的模型服务。候选须引用真实记录，不能修改上游、越界文件或未完整提供的内容。

检查覆盖 JSON／TOML／Python／Shell 语法（Python 脚本使用原生 Tree-sitter 解析，不调用解释器）、Skill 基本元信息，以及资源根目录可选的 `evals.json` 文本约束：

```json
{"required": ["Verify result"], "forbidden": ["Repeat unchanged read"]}
```

这些检查能发现格式或必要内容被破坏，不能证明真实业务执行更快或更好。因此候选仍标记部分验证，以 Draft MR 交人工审核，不自动合并、直接应用或运行机器人／生产工具。没有有效变更不创建 MR；提交失败的记录留待重试。

## 诊断

```sh
./sm status
./sm sync
./sm cycle
systemctl --user status sm-web.service sm-cycle.timer
journalctl --user -u sm-web.service -u sm-cycle.service
```

`sync` 只同步应用；`cycle` 另外消化记录和提出优化 MR。重入由本地锁拒绝。凭据保留在原有工具环境中，可通过 `~/.config/sm/environment` 为用户服务补充必要环境变量；不写入资源仓库。停止自动任务可执行 `systemctl --user disable --now sm-cycle.timer sm-web.service`，已应用资源不会因此被删除。

本地状态位于 `~/.local/state/sm/`，含私有数据库、事件记录和提案副本；服务定义在 `~/.config/systemd/user/sm-*`。这些不需要加入 Git。事件记录暂不自动清理，需要保留审阅所需证据后由用户清理。

## 维护清单

前端提交的是下面格式的清单差异，日常管理不要求直接编辑它。

```json
{
  "projects": {"app": "~/repo/app"},
  "resources": [{
    "id": "review", "kind": "skill", "origin": "self",
    "scope": "project", "project": "app", "payload": "resources/review"
  }]
}
```

Skill 使用单一 `payload`，投放到用户或项目的 `.agents/skills/<id>/`，由支持该标准的客户端发现。其余类型使用 `agents` 和 `payloads`：如 `agents: ["codex"]`、`payloads: {"codex": "resources/example/codex"}`。不得把客户端原生配置差异扩散成多份 Skill。

Codex Hook 使用 `hooks.json`，Kimi Hook 使用 `config.toml`；Hook 可通过 `SM_RESOURCE_DIR` 引用配套脚本。MCP 只接管自身服务条目，规则使用有边界的 SM 文本块。Kimi 原生项目级 Hook 暂无对应能力，SM 明确报告“不支持”，不会扩大为用户级。具体原生路径见[实施计划](superpowers/plans/2026-10-21-sm-phase-one.md)。


## 开发检查

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
npm ci --ignore-scripts
npm run check
npm run build
git diff --exit-code -- assets/static/app.js
```

核心和 CLI 为 Rust；浏览器逻辑为严格模式 TypeScript，并检查未使用的局部变量和参数，类型声明位于 `frontend/types.d.ts`。页面、静态文件、分析提示和服务模板嵌入二进制。修改 TypeScript 后须同时更新已提交的 `assets/static/app.js`；不需要额外前端框架。

GitLab 在每个 MR 和主线提交运行上述检查及 release 构建，构建成功的 Rust 任务保留可执行文件七天。CI 不运行需要个人凭据的模型请求或桌面浏览器交互，也不初始化真实用户服务。

隔离浏览器验收可运行 `cargo run --example preview -- <临时仓库> <临时home> <端口> [状态数据库]`，然后执行 `SM_BROWSER_URL=http://127.0.0.1:<端口> npm run test:browser`。需要预装 browser-harness；回归驱动为 TypeScript，外部 Browser Harness 负责浏览器连接。预览的 MR 页面明确标记为演示，不会提交真实远端。
