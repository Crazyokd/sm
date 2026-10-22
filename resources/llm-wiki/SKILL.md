---
name: llm-wiki
description: 维护/查询 LLM 管理的知识库（如 easygo/easygo_wiki）。五操作：query/compile/ingest/lint/audit。用于在结构化 wiki（concepts/entities/summaries + YAML frontmatter + wikilinks + mermaid/KaTeX）里检索产品/规格/工程知识、把原始资料收敛成页面、审计一致性。触发词：wiki、知识库、compile、ingest、query、lint、audit、产品规格、架构总览。
---

# LLM Wiki 操作手册

适用仓库：`easygo/easygo_wiki`（产品实现知识库）。**每次会话先读 `CLAUDE.md` 与 `wiki/index.md`**，再按操作执行。语言 zh，中性产品工程口吻，不做营销夸张。

## 结构约定

- `wiki/concepts/<topic>/<页面>.md`：概念页（一个页面一个稳定主题）；一级主题固定 `product-definition/ engineering-foundation/ delivery-validation/ ai-assistant/ software-requirements/ knowledge-method/`，每主题有 `index.md` 总览。
- `wiki/entities/<实体>.md`：产品名等实体。`wiki/summaries/<english-kebab>.md`：摘要。
- 每页 YAML frontmatter：`title/type(concept|entity|summary)/created/updated/sources/tags`。
- 页面间用 `[[concepts/...|显示名]]` 路径式 wikilink；首现重要概念/实体必须建链。
- 图表一律 mermaid；公式一律 KaTeX；不做 ASCII art。
- 原始资料镜像 `raw/notes/`（product/technology/requirements/competitors 分桶）；大文件用 `raw/refs/` 指针。`raw/notes/competitors/` 只作技术参照不写商业化结论。
- 矛盾内容并列呈现并在 `Open research questions` 登记，不静默抹平。

## 五操作

### query（检索）
目标：基于 wiki 回答问题，优先页面结论而非重新推导。
1. 读 `CLAUDE.md`（规范）+ `wiki/index.md`（地图）。
2. 按 wikilink 定位相关概念/实体/摘要页；逐页读正文。
3. 引用答案时给页面路径；页面缺失/过时 → 登记到 `Open research questions` 或建议 ingest。
4. 区分口径：**已确认实现 / 设计已定未落地 / 待验证假设** 三态，不混写。

### compile（收敛）
目标：把分散原始资料收敛为稳定的概念/摘要页。
1. 从 `raw/notes/` 对应分桶读原始资料，抽取与产品/工程相关的事实。
2. 建/更新概念页：一个主题一页，frontmatter 的 `sources` 列出所用 raw slug。
3. 摘要页用英文 kebab slug；实体页放产品名。
4. 每操作完成后在 `log/YYYYMMDD.md` 追加条目。

### ingest（摄入）
目标：把新原始资料与新建页面并入 wiki。
1. 小型文本资料镜像到 `raw/notes/<bucket>/`；大文件/二进制用 `raw/refs/` 指针。
2. 为该资料建摘要页并链到相关概念页；更新 `wiki/index.md` 与主题 index。
3. 标注 `updated` 与 `sources`。

### lint（一致性检查）
目标：检查结构一致性。
1. frontmatter 字段齐全且合法；`type` 与目录匹配。
2. wikilink 目标存在（无死链）；首现概念已建链。
3. 图表为 mermaid、公式为 KaTeX；无 ASCII art 混入。
4. 违反项列出 `file: 问题`，修复或登记。

### audit（审计）
目标：页面 vs 原始资料 vs 现状 的核对。
1. 抽查页面结论是否被 `sources` 支持；标注无源/过时结论。
2. 维护 `Audit backlog` 与 `Research gaps`（待 ingest 源）。
3. 与实现/现场不符处：列冲突点、来源页、暂定判断，进 `Open research questions`。

## 与其他资产关系

- 本 skill 操作的是**外部知识库仓库**（easygo/easygo_wiki），产物通过该仓库 MR 提交。
- organism 的共享记忆/guide 是**执行经验**（交付模式/已修问题/成本），与本 wiki 的**产品/规格/架构知识**互补；执行任务可读 wiki 取产品意图，不把执行经验倒灌进 wiki 的产品口径页（除非属 delivery-validation）。
