# Codex 项目入口

本文件是 Codex 进入本仓库时的根入口。

详细规则、项目上下文和工具说明统一放在 `.agents/` 目录，避免多处规则互相冲突。

## 启动阅读顺序

Codex 开始处理任务前，必须先阅读：

1. `.agents/README.md`
2. `.agents/rules/codex_rules.md`
3. `.agents/system/architecture.md`
## 核心原则
- 先读文件，再改代码。
- 小步修改，直接服务当前目标。
- 不要硬编码绝对路径；所有 path 路径都以感知仓库根目录作为起点。
- 不自动提交 Git，除非用户明确要求。
- 不修改 `third_party/` 下第三方代码。
- 能验证就验证，并说明验证在哪台机器执行。
