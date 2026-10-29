# 上游来源归属修正

本次核对 101 条自维护声明及其 178 个源文件。没有找到上游不代表证明原创；自维护说明当前修改责任，不说明历史作者。

## 回归上游更新

- EasyGo App 的 openspec-apply-change、openspec-archive-change、openspec-explore、openspec-propose 与 OpenSpec 1.2.0 原生生成结果逐字节一致。现通过 Rust `source-openspec` 调用 `@fission-ai/openspec@latest` 的原生初始化器，在临时目录只选择通用 agents 输出，再物化这四个完整 Skill。作用域仍为 easygo-app。临时配置、数据和 Codex 目录全部隔离；不向用户级恢复旧 Prompt，也不修改项目 OpenSpec 配置。
- 四个 source-command-opsx-* 仅包装了同一上游命令正文，现移除重复声明与副本，保留原生 Skill。
- 两份项目级 remix-icon 与 [einverne/agent-skills/vendor/remix-icon](https://github.com/einverne/agent-skills/tree/main/vendor/remix-icon) 只差末尾换行。[镜像来源说明](https://github.com/einverne/agent-skills/blob/main/vendor/README.md) 明确其来自 mrgoonie/claudekit-skills。原仓现只保留 web-frameworks/references/remix-icon-integration.md，独立 Skill 已整合；因此登记实际存在的公开镜像快照来源，跟随该镜像目录，保留两个原项目范围，不扩大为 web-frameworks 订阅。镜像更新不保证与原仓同步。
- user-hook-codex-reference 和 user-hook-codex-memory-persistence-reference 回归现有 ECC 来源。codex-hooks.json 及两个 memory-persistence 文件与 ecc-universal 2.2.3 发布包逐字节一致；通用 README 属于较早 ECC 文档快照。正文不再存入自维护目录，由既有原生来源生成器按声明路径更新。202 个共享运行文件与其他消费者声明保持原样。

## 本地改写与来源记录

- improve-codebase-architecture：此前将 mattpocock/skills 的本地改写继续当作自维护资源保留。2026-10-29 来源复核后纠正为跟随原上游，移除本地改写正文及来源副本；随同登记原件明确调用的 codebase-design、grilling、domain-modeling，各项独立分类，共享现有 Matt Pocock 来源抓取。保留上游原生 agents/openai.yaml 中禁止隐式调用的策略。
- create-subagent：通过审核过的历史 Cursor 原件指纹确认派生来源。此前改写为 Codex 说明并保留；现按用户排除 Cursor 派生资源的要求清退这份本地改写，不建立 Cursor 订阅。现有 Agent 定义与原生能力不随这个 Skill 的移除而删除。
- 用户级 AGENTS.md：前四节与 multica-ai/andrej-karpathy-skills 的 CLAUDE.md 去掉标题后一致，补充引用。后续 CodeGraph 维护块与图示、SM 规则继续作为本地组合规则维护。

## 插件声明

七条 Codex/Kimi 插件策略的 self 属于个人订阅与启停声明；实际插件已登记外部 source 并交给原生安装器。目录和详情页面明确显示“订阅声明”，详情列出插件上游，避免将插件内容描述为用户原创。来源、作用域、原生安装及启停策略不因展示改变。
