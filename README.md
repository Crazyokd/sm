# SM

以 Rust 和严格 TypeScript 实现的 Linux 本地 AI 资源管理：可视化维护、跨机器恢复、夜间同步与基于使用记录的改进提案。

- 管理 Skill、Hook、规则、Agent 定义和 MCP 配置，区分来源与用户／项目作用域。
- Skill 只有一份通用内容，Codex、Kimi 通过共享目录加载。
- 自建修改经 MR 审核合并后再同步生效；上游定时直接跟随，不参与本地优化。
- 日常轻量记录，夜间提出带证据和验证边界的 Draft MR，人工决定是否合并。

```sh
git clone http://git.standard-robots.com/yicao/sm
cd sm
./sm init
```

前端：<http://127.0.0.1:8765>。构建需要 Rust 1.88+、C 编译器、Git 和 systemd 用户会话；复用本机 Codex／Kimi 和 GitLab 认证。管理程序运行不依赖 Python；清单中的上游 Skill 恢复需要 Node.js 20+ 和 npm/npx。仅支持当前 Rust 数据格式；不提供旧安装状态的迁移或回退兼容。

[使用说明](docs/usage.md) · [验证结果与限制](docs/verification.md) · [需求](docs/superpowers/specs/2026-10-21-sm-design.md) · [Rust 迁移计划](docs/superpowers/plans/2026-10-21-sm-rust.md)

[首批资源与接管说明](docs/resource-adoption.md)：整合 easygo-skills 的自建资源、项目归属和第三方来源。

![资源目录](docs/screenshots/resources.png)
