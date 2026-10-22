# Codex 项目入口

本文件是 Codex 进入 EasyGo 外层感知仓库时的根入口。

这个仓库的重点不是算法源码本身，而是 EasyGo 感知对外接口、桥接包、已验证 runtime 快照、正式 deb 发布和热开发入口。算法源码细节继续进入 `perception_platform/` 后按它自己的 `AGENTS.md` 和 `.agents/` 规则处理。

## 启动阅读顺序

Codex 开始处理本仓库任务前，必须先阅读：

1. `.agents/README.md`
2. `.agents/rules/codex_rules.md`
3. `.agents/system/repository_map.md`
4. 按任务类型继续读取：
   - 发布 / deb / CI：`.agents/workflows/release_and_ci.md`
   - 热开发 / runtime 固化：`.agents/workflows/runtime_snapshot.md`
   - 构建 / 验证命令：`.agents/tools/build_and_verify.md`
   - 已知坑和协作经验：`.agents/memory/known_issues.md`、`.agents/memory/lessons_learned.md`

## 核心原则

- 先读文件，再改代码。
- 小步修改，直接服务当前目标。
- 不自动提交 Git，除非用户明确要求。
- 不把 `perception_platform/` 源码构建误认为正式发布流程。
- 不把 `perception_runtime/` 当作临时构建目录随手覆盖；只有车上验证 OK 后才固化。
- 不修改运行时大产物，除非任务明确要求更新 runtime 快照。
- 能验证就验证，并说明验证在哪台机器执行。
