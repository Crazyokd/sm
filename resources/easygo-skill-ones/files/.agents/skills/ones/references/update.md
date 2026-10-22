# 更新

状态、负责人、优先级或通用字段更新时，用这个流程。

## 解析意图

常见形式：

- `<ID> 状态为 <状态名>`
- `<ID> 转给 <姓名>`
- `<ID> 优先级 <级别>`
- `关闭 <ID>`
- `<ID> <字段>=<值>`

## 必要读取

1. task info
2. 涉及负责人时读取 members
3. 涉及状态时读取 task statuses
4. 涉及状态时读取可用 transitions

## 状态更新

状态流转必须走 transitions：

1. `GET /team/UNrQ5Ny5/task/<task_uuid>/transitions`
2. 用用户指定的状态名去模糊匹配 transition 名称
3. 如果没有安全匹配，问一个简短问题并列出可选流转
4. `POST /team/UNrQ5Ny5/task/<task_uuid>/transit`

如果同一次请求还改负责人或优先级，先 transit，再 `tasks/update`，避免工作流副作用把目标字段覆盖掉。

**transit 后必须紧跟一次 `tasks/update`，把 `assign` 回写为 ONES token 用户 UUID**（缺陷/任务工单同时回写 `field_values` 中的 `HiRJZNR7`）。除非用户明确指定他人，否则不要让 transit 副作用把解决人换走，否则统计无法归属。详见 [common.md](./common.md) 「默认负责人 / 问题解决人」。

## 负责人和优先级

负责人：

- **仅当用户明确说"转给某某"或指定 UUID 时才改 `assign`**。日常状态/优先级/描述更新，不要顺手改 `assign` 或 `HiRJZNR7`。
- 从 members 列表里模糊匹配
- 匹配出多人时，让用户选

优先级 UUID：

- P0 / 紧急 / 最高 -> `N3f3Lpa7`
- P1 / 重要 / 较高 -> `X4zCjKmD`
- P2 / 普通 -> `4Fox75xJ`
- P3 / 较低 -> `7uf9Egvm`

## 通用字段更新

使用：

```json
{"tasks": [{"uuid": "<task_uuid>", "<field>": "<new_value>"}]}
```

成功校验要严格：响应里应有 `tasks`，而不是只有 `bad_tasks`。

## 输出

用简短摘要说明旧值和新值。
