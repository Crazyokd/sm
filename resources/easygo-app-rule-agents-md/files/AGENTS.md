# AGENTS.md

## 适用范围

本文件适用于 `easygo-app` 仓库。若当前仓库位于 EasyGo 主仓
`src/easygo-app` 下，先读取主仓 `../../AGENTS.md`；主仓规则优先约束
容器、跨仓协作、部署和 MR。

## 工作原则

- 先读后写，先确认调用链、边界和现有实现。
- 只修改需求相关内容，不做无关重构或顺手清理。
- 修复问题时先建立最小可复现验证；修改后运行与影响范围匹配的检查。
- 反馈时区分已验证项、未验证项和远端 CI 状态。
- Rust 模块使用 `foo.rs` + `foo/` 子模块布局，不新增 `mod.rs`。

## 标准命令

优先通过 `just` 执行项目命令：

```bash
just setup          # 初始化 Git hooks
just precommit      # 完整检查：check、typos、格式化、test、lint
just fmt-staged     # 只格式化 staged 的 Rust/TOML 文件
just fmt            # 格式化并修复可自动修复的问题
just test           # cargo nextest run
just lint           # fmt、clippy、shear、taplo 检查
```

`pre-commit` 只执行 `git diff --cached --check`，不会因为编译、测试或
lint 失败而阻止提交。完整的 `just precommit` 应在 MR 转 Ready 前运行，
MR CI 仍是合并门禁。

## 代码约束

- 遵守 workspace lint：禁止 `unwrap()`、`expect()` 和直接数组索引。
- 使用现有类型、辅助函数和模块边界，不为单一用途新增抽象。
- 使用 `typos`、`taplo` 和 `cargo shear` 保持检查一致。
- 新增或修改代码注释使用中文。

## MR 与提交

- 未经用户主动要求，不自动执行 `git commit`、`push` 或创建分支。
- 有可展示的阶段性进展即可先提交、推送并创建 Draft MR，不要求首次提交
  前完整检查通过。
- Draft MR 创建后，编译、测试或 lint 失败时追加普通提交并推送；不要
  amend 或强推已发布 MR。
- 最新提交的本地 `just precommit` 和 MR CI 都通过后，再将 Draft MR 转为
  Ready。
- commit message 使用中文描述变更内容。
- AI 协作在 commit message 和 MR 描述 trailer 中披露：
  `Assisted-by: <实际工具>:<本次可信运行信息中的模型标识>`。
  不照抄旧型号或猜测身份；模型未确认时只写实际工具，并说明模型未确认。
- AI 工具不能作为作者或 `Signed-off-by`；签署主体必须是实际负责的人。

## 验证与协作边界

- 在独立工作树操作；恢复工作区不等于撤回 MR 中的提交，需核对当前 MR 差异并在授权后用普通 revert 撤回。
- Rust 检查使用已有 `easygo-app-checks` 入口，记录实际检出、HEAD、工作区差异和退出码；额外修改过的验证副本不能证明原 MR 提交通过。
- 当前远端结论先 fetch 并核对精确 SHA；涉及时间、范围或饱和边界时独立计算总量并断言验收上限。
- 替换接口前检查目标正式接口包已有的兼容 Rust crate；避免另写一套手工绑定。CI 选测范围以 `scripts/test_changed.py` 和当前 CI 配置为准，不能识别的代码应明确失败。
