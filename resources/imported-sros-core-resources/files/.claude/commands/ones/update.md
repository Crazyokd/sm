# ONES 更新工作项

更新工作项的状态、负责人、优先级等字段。

## 参数

$ARGUMENTS

解析规则：
- `<工作项ID> 状态为 <状态名>` → 更新状态
- `<工作项ID> 转给 <人名>` → 变更负责人
- `<工作项ID> 优先级 <级别>` → 修改优先级
- `关闭 <工作项ID>` → 状态转为已关闭/已完成
- `<工作项ID> <字段>=<值>` → 通用字段更新

## 前置依赖

认证流程见 `common.md`。

---

## 执行流程

### Phase 1：认证 + 获取工作项信息

按 `common.md` 认证，然后并行获取：
- 工作项详情：`GET /team/UNrQ5Ny5/task/<ID>/info`
- 成员列表：`GET /team/UNrQ5Ny5/members?limit=500`（负责人操作时需要）
- 状态列表：`GET /team/UNrQ5Ny5/task_statuses`（状态操作时需要）

### Phase 2：解析操作意图

从参数中识别操作类型和目标值：

#### 状态更新

**状态转换需要通过 transition API，不能直接设置 status_uuid。**

1. 获取当前工作项可用的状态转换：
   ```
   GET /team/UNrQ5Ny5/task/<task_uuid>/transitions
   ```
   返回：
   ```json
   {"transitions": [
     {"uuid": "NBn3Q4d3", "name": "开发中", "start_status_uuid": "TpD5FvvJ", "end_status_uuid": "K7mLx1ej"},
     ...
   ]}
   ```

2. 用户指定的状态名与 transition 的 `name` 模糊匹配（包含即匹配）
3. 找不到匹配时，列出可用转换让用户选择（AskUserQuestion）
4. **触发转换**：
   ```
   POST /team/UNrQ5Ny5/task/<task_uuid>/transit
   Body: {"transition_uuid": "<matched_transition_uuid>"}
   ```
   成功返回 `{"code": 200, "errcode": "OK"}`

> 注意：`POST /tasks/update` 设置 `status_uuid` **无法跨工作流阶段**，必须用 transit API。

#### 负责人变更

> **前提**：仅当用户**明确**说"转给某某"或指定 UUID 时才改 `assign`。日常更新（状态、优先级、描述等）**不要**顺手改 `assign` 或问题解决人字段（`HiRJZNR7`），默认值是 ONES token 对应用户，详见 common.md「默认负责人 / 问题解决人」。

1. 从成员列表中按姓名模糊匹配（包含即匹配）
2. 多个匹配时列出让用户选择
3. 字段：`assign` = 目标用户 UUID

#### 优先级修改

| 关键词 | UUID |
|--------|------|
| 最高/紧急/P0 | `N3f3Lpa7` |
| 较高/重要/P1 | `X4zCjKmD` |
| 普通/P2 | `4Fox75xJ` |
| 较低/P3 | `7uf9Egvm` |

### Phase 3：确认并执行

展示变更摘要（当前值 → 目标值），AskUserQuestion 确认后执行：

**负责人/优先级等字段**：
```
POST /team/UNrQ5Ny5/tasks/update
Body: {"tasks": [{"uuid": "<task_uuid>", "<field>": "<new_value>"}]}
```

**状态转换**：
```
POST /team/UNrQ5Ny5/task/<task_uuid>/transit
Body: {"transition_uuid": "<transition_uuid>"}
```

> 状态和其他字段需分开调用（transit + update）。先执行 transit（可能触发工作流副作用如变更负责人），再执行 update 覆盖其他字段。
>
> **transit 后必须立即用 `tasks/update` 把 `assign` 回写为 ONES token 用户 UUID**（缺陷/任务工单同时回写 `field_values` 中的 `HiRJZNR7`），防止工作流副作用把解决人换成他人导致统计丢失。仅当用户明确指定他人时才用他人 UUID。

验证响应包含 `"tasks"` 键（非 `"bad_tasks"`）。

### Phase 4：输出结果

```
#<ID> 已更新：<字段> 从 "<旧值>" → "<新值>"
```

## 批量更新

支持一次更新多个字段：
```
/ones 更新 148707 状态为已解决 转给张三
```

解析为多字段，合并到一次 API 调用：
```json
{"tasks": [{"uuid": "xxx", "status_uuid": "yyy", "assign": "zzz"}]}
```
