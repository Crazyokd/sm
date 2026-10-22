---
name: ones
description: 处理 Standard Robots 的 ONES 工作项。适用于查看、列表、创建、更新、评论、分析、修复、实现 ONES 的需求、缺陷、任务等场景，也覆盖 ONES、工单、需求、缺陷、任务、评论、更新、分析、修复、实现，以及 view、create、update、comment、fix、implement 等触发词。
---

# ONES

用于 Standard Robots 的 ONES 操作。

优先使用随 skill 提供的脚本，不要每次手写 `curl`：

- `python3 scripts/ones_api.py request GET /team/UNrQ5Ny5/task/149579/info`
- `python3 scripts/ones_api.py graphql '{ projects { uuid name } }'`
- `python3 scripts/ones_create_task.py "清理测试中的opencv依赖" --type task`

## 认证

认证来源优先级（`scripts/ones_api.py` 按此处理）：

1. `ONES_USER_ID` + `ONES_AUTH_TOKEN`
2. `ONES_UUID` + `ONES_TOKEN`
3. `ONES_TOKEN_JSON`（格式 `{"uuid": "...", "token": "..."}`，兼容 GitLab CI 变量）
4. `~/.ones_token`（格式 `{"uuid": "...", "token": "..."}`）
5. 失效或缺失时自动刷新：
   - 有 `ONES_EMAIL` + `ONES_PASSWORD` → 走密码登录（CI / 无 GUI 场景）
   - 否则 → 调用 `scripts/ones_refresh_token.py`，从持久化 Chromium profile
     抽取 `ones-uid` / `ones-lt` cookie（飞书 SSO 默认路径，会话还在时无需扫码）

手动刷新：`python3 scripts/ones_refresh_token.py`（首次或会话过期会弹窗扫码；
`HEADLESS=1` 走无界面刷新）。

## 路由

- 纯数字 ID 或 task UUID：看 [references/view.md](./references/view.md)
- 自然语言筛选或多个 ID：看 [references/view.md](./references/view.md)
- 创建或新建工作项：看 [references/create.md](./references/create.md)
- 状态、负责人、优先级或通用字段更新：看 [references/update.md](./references/update.md)
- 查看评论或新增评论：看 [references/comment.md](./references/comment.md)
- 问题分析：看 [references/analyze.md](./references/analyze.md)
- 从 ONES 问题走到代码修复：看 [references/bugfix.md](./references/bugfix.md)
- 从 ONES 需求走到实现：看 [references/implement.md](./references/implement.md)
- 发布管理 / 列发布工作项 / 取 publish_content 关联清单：看 [references/release.md](./references/release.md)

## 使用规则

- 调 API 前先读 [references/common.md](./references/common.md)。
- 写操作只有在项目、类型、字段值不明确且可能改错单时，才额外问一个简短问题。
- 批量处理先列出候选项；如果用户没有明确范围，再确认处理哪些。
- 默认不要用 sub-agent。只有用户明确要求委派或并行 agent 时才用。
- 状态流转必须走 transit API，不能只靠 `tasks/update`。
- 评论、流转、附件和大多数更新场景，如果用户给的是数字 ID，先查 task info 拿到 `task_uuid`。
- 回写分析报告、处理结论或状态说明到 ONES 描述/问题详情时，禁止覆盖已有内容；必须先读取当前 `desc_rich`，在原内容后追加 AI 内容。如果无法安全保留原内容，改用评论 `send_message`，不要更新 `desc_rich`。
- **负责人 / 问题解决人默认 = ONES token 用户 UUID**：创建工单的 `owner`/`assign`、缺陷或任务工单的 `HiRJZNR7` 字段、以及 transit 后必须紧跟的 `tasks/update`，都要用 token 对应用户。否则 ONES 工作量统计无法归属当前帐号。仅当用户明确指定他人时才用他人 UUID。详见 [references/common.md](./references/common.md) 「默认负责人 / 问题解决人」。

## 参考文件

- 公共 API、认证、常量：[references/common.md](./references/common.md)
- 查看和列表：[references/view.md](./references/view.md)
- 创建：[references/create.md](./references/create.md)
- 更新：[references/update.md](./references/update.md)
- 评论：[references/comment.md](./references/comment.md)
- 分析：[references/analyze.md](./references/analyze.md)
- 修复闭环：[references/bugfix.md](./references/bugfix.md)
- 需求实现闭环：[references/implement.md](./references/implement.md)
- 发布管理：[references/release.md](./references/release.md)
