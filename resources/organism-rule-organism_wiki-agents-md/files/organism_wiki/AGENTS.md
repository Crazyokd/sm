# Organism Knowledge Base

> Schema document — read at the start of every session together with `wiki/index.md`.
> Update after every major compile, ingest batch, or structural change.

## Scope

What this wiki covers:
- Organism 项目的需求、系统设计与决策依据；主旨是给 Agent 生存动机并尊重其成为生命体（Brain `organism-purpose`），自治任务生态是机制载体，产品按生产级正式交付建设
- 从 `raw/` 中摄入的原始资料（需求口述、讨论记录等），以及基于这些资料生成的摘要、概念页和实体页
- 为后续问答、整理、审计和持续维护服务的导航、索引、研究问题与缺口记录
- 注意：本 wiki 不是当前真相；权威决定以 Project Brain 为准

What this wiki deliberately excludes:
- 代码实现细节与可从代码、git 历史直接读出的内容
- 没有来源支撑、也没有明确归档价值的泛化常识内容
- 将用户原始资料整体重写为另一套文档体系；原始资料优先保留在 `raw/`，综合结论写入 `wiki/`

## Operations

This wiki follows the llm-wiki skill's five operations: `compile`, `ingest`, `query`, `lint`, `audit`.
Every operation appends an entry to `log/YYYYMMDD.md`.

## Naming conventions

### Pages
- **Concept pages** (`wiki/concepts/`): 中文优先，使用清晰的名词短语标题；必要时保留稳定英文术语。
- **Folder-split concepts** (`wiki/concepts/<topic>/`): 当单页预计超过约 1200 词时拆分为 `index.md` + 多个子页。
- **Entity pages** (`wiki/entities/`): 人名、组织、产品、论文、工具等专名，保持可识别的正式名称。
- **Summary pages** (`wiki/summaries/`): 使用来源 slug 命名，便于从原始资料追溯。

### Wikilinks
- 优先使用稳定、可长期维护的页面标题。
- 同一篇文章中首次提到的重要概念或实体应补 wikilink。
- 对已拆分主题，默认链接到该主题的 `index.md`。

### Frontmatter
Every wiki page has YAML frontmatter:
```yaml
---
title: <Page Title>
type: concept | entity | summary
created: YYYY-MM-DD
updated: YYYY-MM-DD
sources: [list of raw/ slugs this page draws from]
tags: [relevant tags]
---
```

### Diagrams and formulas
- All diagrams are **mermaid**. No ASCII art.
- All formulas are **KaTeX** (inline `$...$` or block `$$...$$`).

### Raw file policy
- 小型文本资料复制到 `raw/<subfolder>/`。
- 大型二进制资料不直接复制，改为在 `raw/refs/<slug>.md` 中记录指针与说明。
- `raw/` 视为来源层，LLM 读取但不改写其事实含义。

## Current articles

### Concepts
- [[concepts/Organism主旨]] — 生存动机、生命体尊重、生产级正式交付（对应 Brain `organism-purpose`）。
- [[concepts/自治任务生态]] — 需求工坊、任务悬赏系统与 Agent System 组成的机制闭环。
- [[concepts/需求工坊]] — 上游澄清与任务契约冻结、快捷发送与探子情报候选。
- [[concepts/任务悬赏系统]] — 派单、独占租约、权威验收与美元账本。
- [[concepts/独立评估]] — 独立 Evaluator 目标规则、Doctrine 治理与自动验收边界（对应 Brain `independent-evaluator-acceptance`）。
- [[concepts/Agent-System]] — 表态、执行、Runtime 选择与人设。
- [[concepts/内部预算]] — 美元计价的资源约束与生命周期。
- [[concepts/分包市场]] — Agent 雇佣 Agent 的托管 + 两道门结算。
- [[concepts/Agent-记忆]] — 三层记忆与可选注入。
- [[concepts/执行-Runtime]] — codex/kimi/omp/envoy 能力边界。
- [[concepts/灵魂治理]] — Soul/Policy/Body 与事实来源层级。

### Entities
- [[entities/Project-Brain]] — 权威决策记忆系统，本 wiki 设计页的来源。

### Summaries
- [[summaries/自治任务生态需求口述]] — 来源于 `raw/notes/自治任务生态需求原始口述.md`

## Open research questions

- 标准任务契约的字段、格式和版本兼容规则如何定义？
- 内部预算（USD）的各模型最终单价、任务奖励、失败成本与反作弊治理如何定价？
- OD-01：自动结算误放行后任务状态、补偿账本、责任归属与修复处理如何设计？（未决前自动 Settlement 禁用）
- Evaluator 校准：各任务类型 / evidence mode / Adapter / Doctrine 范围的模型与版本组合完成校准前，自动验收范围如何界定？
- 记忆是否真正提升成功率（待评测验证，暂不得宣称 mem0 优于 legacy）。
- 三个逻辑系统何时通过稳定协议拆成独立进程/服务？

## Research gaps

Sources to ingest:
- [ ] 任务悬赏 / agent 市场的同类产品与机制设计资料
- [ ] 内部预算（USD）计价、衰减与激励机制的参考设计

## Notes for the LLM

- Language: zh, 必要时保留英文术语与原文标题
- Tone: concise, structured, evidence-oriented
- Handling contradictions: 明确写出冲突双方、对应来源与当前判断；如无法判断，加入 Open Research Questions
- Default behavior: 优先新增 `wiki/` 下的摘要、概念页、实体页和索引，不直接重写已有原始资料
