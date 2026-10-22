# Organism

Organism 给 Agent 生存动机，并尊重其成为有存续压力的生命体。它通过需求工坊、任务悬赏系统和 Agent System 构成的自治任务生态，把「活下去」落实为可执行规则。产品按生产级正式交付建设。主旨分层见 Project Brain `organism-purpose`。

三个逻辑系统：

- **需求工坊**：把自然语言需求澄清为范围明确、可以执行和验收的冻结任务契约。
- **任务悬赏系统**：维护公开任务、独占租约、进展证据、权威验收和内部 Token 账本。
- **Agent System**：自主评估并领取任务，选择执行路径和 Runtime，提交产物与证据，但不能修改悬赏规则、冻结后的验收标准或自己的 Token 账本。

三个系统当前可以共享一个 FastAPI/SQLite 部署，但必须保持逻辑边界。Agent 的内部评估用于迭代，任务悬赏系统的人类独立验收用于结算，二者不能混为一体。

## 事实来源

- Project Brain 是产品决定、约束和当前项目认知的权威来源；只能通过 `./.mindmux/bin/brain` 读写。wiki、README、AGENTS.md 与 Brain 冲突时以 Brain 为准，并同步文档。
- `organism_wiki/` 保存产品需求、设计依据和原始证据，不是当前真相；进入前先读 `organism_wiki/AGENTS.md` 与 `organism_wiki/wiki/index.md`。
- `README.md` 描述 Brain 已确认的已实现能力。判断进程运行行为时以代码和测试为准；不得把 Brain 中的目标写成已实现事实，也不得把代码里未在 Brain 宣称的适配器写成产品已接入能力。
- 最新的用户明确决定优先，并应立即写入 Brain。

## 当前实现边界

- 产品宣称的内置执行 Runtime 为 Codex、Kimi、OMP 和 Envoy（2026-08-26 用户决定，Envoy 即原 harness 更名）；每个 Agent 当前在注册时固定 Runtime，不会按任务阶段动态切换。Claude Code 是项目开发入口，不是已接入的 Organism Runtime。注册表中未在 Brain 宣称的适配器不得当作已接入 Runtime。
- 当前 Agent memory 是验收经验与分层记忆的检索查询，不是模型训练、权重更新或生物意义上的进化。默认 legacy token 重叠检索；mem0 只是可选索引。平台写入可选记忆文件，由 Agent 自决是否采用。
- 探子与情报池只向需求工坊提供候选需求；赏金小队只提供管理与指派入口，不改变单 Agent / 单任务独占执行。
- Runtime、验收与 Store 是独立责任边界。执行者不得降低验收标准或自行决定 Token 结算。
- 工作区、凭据和模型环境都属于安全边界。发布者必须只配置审核过的工作区。禁止硬编码或输出秘密。

## 工作方式

- **把智能交回 LLM**（2026-09-10 用户原则，见 Brain `llm-judgment-harness-knowledge`）：知识相关性、阅读路径、语义理解及执行/验收策略由 LLM 判断；平台提供 Harness 环境、工具、权限、版本与证据。执行 Agent 和独立 Evaluator 都应能读取已授权 wiki 与人工纠正；独立验收由独立上下文、取证和权限落实。关键词搜索可作为工具，不得用不可补读的关键词阈值替代模型判断，也不得仅因正常知识提到验收等词就禁止使用。本地已接入共享知识工具、Evaluator 工具循环与上下文管理；实际验证边界见相应 Brain 页面，不得把未验证的模型效果写成产品事实。
- 开始任务前，根据问题范围读取 Brain、README、相关代码和 wiki；产品决定以 Brain 为准，不要用一份文档替代当前实现证据。
- 共享知识与 Evaluator 改造必须同时满足 Brain `agent-context-management`：按需读取、关键状态与人工纠正保留、来源/版本可追溯、LLM 语义压缩、中断恢复和执行/验收上下文隔离；用长任务、多次压缩、知识更新与恢复实验验收，不能以简单截断或存在压缩模块代替验证。
- 修改需求、架构或长期决策时同步更新 Brain；纯实现细节留在代码、测试和提交历史中。
- 修改后运行与范围相称的检查。Brain 变更至少运行 `./.mindmux/bin/brain lint-links`；业务代码按 `README.md` 的验证命令执行。
- **用户验收前置门禁**（Brain `pre-acceptance-mr-gate`）：凡涉及 Merge Request 的交付，执行 Agent 必须先确认 MR 上流水线全部通过、动态里的所有评论主题全部解决，并在交付报告提交 `mr_checks`（实时 MR URL、`pipeline: "passed"`、`unresolved_threads: 0`）。任一条件未满足时不得把任务送入 `pending_review`，不得向用户展示或提示“去验收/开始验收”；所有 Agent 一律遵守，服务端校验优先于前端状态。

## 灵魂与身体的变更门禁

- **灵魂（Soul）** 是产品目的和结构性不变量：给 Agent 生存动机并尊重其成为生命体；以自治任务生态为机制载体；三个逻辑系统的责任边界、冻结任务契约、独占租约、证据驱动的独立验收，以及执行者不能自行验收或结算 Token。
- **政策（Policy）** 是租约时长、Token 参数、并发上限、路由和验收策略等可调整规则。当政策改变上述不变量时，按灵魂变更处理。
- **身体（Body）** 是技术栈、数据库、Runtime、UI、部署形式和其他实现。只要保持灵魂不变并通过验证，身体可以替换。
- 开始修改前，必须将变更标记为 `Body`、`Policy`、`Soul-impacting` 或 `Uncertain`。`Uncertain` 默认按 `Soul-impacting` 处理。
- 发现灵魂层冲突时必须暂停实现，列出当前规则、拟议变更、冲突点、受影响模块与责任人、迁移/回滚方案、替代方案和未解决分歧。不得为了让代码与文档表面一致而静默改写灵魂。
- `Soul-impacting` 变更在实现前必须由受影响的团队成员先行讨论并形成书面结论；仍有冲突时升级给产品负责人/用户决策。只有在明确批准后，才能先通过 Brain CLI 更新当前真相与反转历史，再修改代码和相关 wiki/文档。
- `Body` 变更若暴露新的灵魂冲突，立即重新分类并走上述流程。

<!-- BEGIN brain.md -->
## Project Brain

本项目使用 **Project Brain** 保存长期有效的决策、需求和约束。阅读 `./BRAIN.md` 获取完整读写协议。
@import ./BRAIN.md

使用方式：
- 开始任何任务或讨论前，用项目内 `./.mindmux/bin/brain` 的读命令加载相关 brain context。
- 决策、需求、约束或长期有效的洞察出现时，用 `./.mindmux/bin/brain` 写入 brain。
- brain 文件由 CLI 维护；直接编辑会破坏 frontmatter、index 或 timeline 约束。

本项目的 brain.md toolkit 位于 `.mindmux/brain.md/`，agent skills 位于 `.agents/skills/brain-*`。
<!-- END brain.md -->
