# ONES API 公共基础

本文件定义 ONES API 的公共认证流程、常量、附件处理和评论写入模板。**所有 `/ones*` skill 共享此定义，不再各自重复。**

> 本文件不直接被用户调用，仅作为其他 ones skill 的引用基础。

---

## 常量

- **ONES_HOST**: `https://ones.standard-robots.com:10120`
- **API_BASE**: `${ONES_HOST}/project/api/project`
- **TEAM_UUID**: `UNrQ5Ny5`
- **GRAPHQL_ENDPOINT**: `${API_BASE}/team/UNrQ5Ny5/items/graphql`
- **Token 缓存文件**: `~/.ones_token`（JSON：`{"uuid": "xxx", "token": "xxx"}`，权限 600）

## 环境变量

- `ONES_EMAIL` — ONES 登录邮箱
- `ONES_PASSWORD` — ONES 登录密码

## 认证流程（乐观策略）

**默认假设 `~/.ones_token` 存在且有效**，直接用缓存 token 发起业务请求。仅在失败时才回退重试。

### 正常路径（无额外请求）

1. 读取 `~/.ones_token`，提取 `uuid` 和 `token`
2. 直接发起业务 API 请求（如获取工作项详情）

### 失败回退

当业务请求返回 **401/403** 或 `~/.ones_token` 不存在时：

1. **加载环境变量**
```bash
source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null
```
若 `ONES_EMAIL` 或 `ONES_PASSWORD` 未设置，停止并提示：
```
请先设置环境变量：
export ONES_EMAIL="your-email"
export ONES_PASSWORD="your-password"
```

2. **重新登录**
```bash
curl -s -X POST "${API_BASE}/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"email\": \"${ONES_EMAIL}\", \"password\": \"${ONES_PASSWORD}\"}"
```
提取 `user.uuid` 和 `user.token`，写入 `~/.ones_token`（chmod 600）。

3. **用新 token 重试业务请求**

### 通用 Header

```
Content-Type: application/json
Ones-Auth-Token: <token>
Ones-User-Id: <uuid>
Referer: https://ones.standard-robots.com:10120
```

## 默认负责人 / 问题解决人

**所有创建、流转、更新操作中涉及负责人（`owner`、`assign`）和"问题解决人"字段（`HiRJZNR7`，type=8）时，默认值必须是 ONES token 对应的用户 UUID（即 `~/.ones_token` 里的 `uuid`，等价于 `ones_api.load_auth()` 返回的 user_uuid）。**

理由：ONES 工单的工作量、解决数等统计按解决人归属，如果不是 token 用户，数据无法计入当前 AI 帐号。

适用规则：

- 创建工单（`tasks/add2`）：`owner` / `assign` 必须填 token 用户；`HiRJZNR7` 字段（缺陷/任务）默认填 token 用户。
- 状态 `transit` 后：transit 可能会副作用变更 `assign`，必须紧跟一次 `tasks/update`，把 `assign`（以及如果是缺陷/任务工单的 `HiRJZNR7` 字段）显式回写为 token 用户。
- 更新工单（`tasks/update`）：除非用户**明确**点名"转给某人"或显式指定他人 UUID，否则不要修改 `assign` 和 `HiRJZNR7`；不得用其他人当默认值。
- 用户明确指定他人时按用户意图执行，不要自作主张覆盖。

## 常用 API

### 获取工作项详情
```
GET ${API_BASE}/team/UNrQ5Ny5/task/<ID>/info
```
返回字段在**顶层**（`data['desc_rich']`），不嵌套在 `task` 下。

### 更新工作项
```
POST ${API_BASE}/team/UNrQ5Ny5/tasks/update
Body: {"tasks": [{"uuid": "<task_uuid>", "desc_rich": "<HTML>"}]}
```
- 必须使用 task UUID（16位），不支持序号
- `desc_rich` 全量覆盖，追加内容需先读后拼
- 请求体必须 `{"tasks": [...]}`（数组格式）

### 获取附件列表
```
GET ${API_BASE}/team/UNrQ5Ny5/task/<task_uuid>/attachments?count=100
```
必须使用 task UUID。

### 批量获取附件下载 URL（推荐）
```
GET ${API_BASE}/team/UNrQ5Ny5/res/attachments?ids=<uuid1>,<uuid2>,...
```
单个附件 API 有瞬态 404 风险，优先用批量 API。签名 URL 有时效性。

### 发送评论
```python
payload = {"uuid": "<8位随机>", "text": "<评论内容>"}
POST ${API_BASE}/team/UNrQ5Ny5/task/${task_uuid}/send_message
```

### 获取评论列表
```
GET ${API_BASE}/team/UNrQ5Ny5/task/<task_uuid>/messages
```
返回 `{"messages": [...]}`。过滤 `type == "discussion"` 为用户评论，`type == "system"` 为系统消息（忽略）。
字段：`uuid`, `type`, `from`（用户UUID）, `text`, `send_time`（微秒）。

### 状态转换查询
```
GET ${API_BASE}/team/UNrQ5Ny5/task/<task_uuid>/transitions
```
返回 `{"transitions": [{"uuid": "...", "name": "阶段名", "start_status_uuid": "...", "end_status_uuid": "..."}]}`。

### 触发状态转换
```
POST ${API_BASE}/team/UNrQ5Ny5/task/<task_uuid>/transit
Body: {"transition_uuid": "<transition_uuid>"}
```
成功返回 `{"code": 200, "errcode": "OK"}`。

> **重要**：直接设置 `status_uuid` 无法跨工作流阶段，必须通过 transit API 触发。transit 可能有副作用（如自动变更负责人），后续需用 tasks/update 覆盖。

### 更新工作项（通用）
```
POST ${API_BASE}/team/UNrQ5Ny5/tasks/update
Body: {"tasks": [{"uuid": "<task_uuid>", "<field>": "<value>", ...}]}
```
支持的字段：`status_uuid`（状态）、`assign`（负责人 UUID）、`priority`（优先级 UUID）、`desc_rich`（描述 HTML）、`field_values`（自定义字段）。

### 成员列表
```
GET ${API_BASE}/team/UNrQ5Ny5/members?limit=500
```
返回 `{"members": [{"uuid": "...", "name": "姓名", ...}]}`，用于姓名 → UUID 解析。

### 项目列表（GraphQL）
```
POST ${GRAPHQL_ENDPOINT}
Body: {"query": "{ projects { uuid name } }"}
```
返回所有项目，用于创建工作项时选择目标项目。

### 项目内工作项类型
```
POST ${GRAPHQL_ENDPOINT}
Body: {"query": "{ project(uuid: \"<project_uuid>\") { issueTypes { uuid name } } }"}
```
查询特定项目支持的工作项类型。

## 附件处理策略

| 类型 | 处理方式 |
|------|---------|
| 图片（image/*） | 下载到 `/tmp/ones:analyze/<ID>/images/`，Read 工具展示 |
| SROS日志（.log.tar.zst） | 下载 → `zstd -d` → `tar xf` → 到 `sros-logs/` |
| Allspark日志（.tar 含 .gz） | 下载 → `tar xf` → `gunzip` → 到 `allspark-logs/` |
| ZIP | 下载 → `unzip -o` → 到 `other-logs/` |
| 大文件（>50MB 非日志） | 仅列出信息，不下载 |

## AI 标记规范

### 评论标识
- 处理中：`[AI Agent] 正在处理中`
- 完成：`[AI Agent] 处理完成`
- 失败：`[AI Agent] 处理失败`

### 报告声明（HTML 报告顶部）
```html
<div style="background:#fff3cd;border:1px solid #ffc107;padding:12px;margin-bottom:16px;border-radius:4px">
<p style="margin:0"><b>声明：本报告由 AI 辅助分析生成，仅供参考。</b></p>
</div>
```

### 已处理检测

判断工作项是否已被 AI 处理（检查评论和描述，取并集）：
- 评论含 `[AI Agent] 处理完成` → 已处理
- 评论含 `[AI Agent] 正在处理` → 处理中
- 描述含 `本报告由 AI 辅助分析生成` → 已处理
- 描述同时含 `分析报告` 和 `根因` → 已处理

检查评论：`GET /team/UNrQ5Ny5/task/<task_uuid>/messages`，筛选 `type == "discussion"`。

### 工作项类型→处理方式映射

| 分类 | 工作项类型 | UUID | 处理方式 |
|------|-----------|------|---------|
| 问题分析 | 现场问题 | `ARCxvBDk` | /ones:analyze |
| | 现场缺陷 | `VGyUpCRi` | /ones:analyze |
| | 客户端问题库 | `Xc7sF3tN` | /ones:analyze |
| | 产品研发部问题库 | `CeENe67t` | /ones:analyze |
| | 疑难问题库 | `2ugm5dXZ` | /ones:analyze |
| | 研发端问题库 | `NPGuDBtg` | /ones:analyze |
| 缺陷修复 | 软件缺陷 | `NuSzzA62` | /ones:bugfix |
| | 缺陷 | `8YtVcwaj` | /ones:bugfix |
| 需求实现 | 软件产品需求 | `GbtzHVF6` | /ones:implement |
| | 需求 | `Roy6EvKA` | /ones:implement |

### GraphQL 列表查询

```
POST ${GRAPHQL_ENDPOINT}
Body: {"query": "{ tasks(filter: {...}, limit: N, orderBy: {createTime: DESC}) { uuid name number createTime issueType { uuid name } status { uuid name } assign { uuid name } } }"}
```

可用 filter 字段：
- `issueType_in: ["uuid1"]` — 按类型
- `createTime_range: {from: <微秒>, to: <微秒>}` — 按时间
- `status_in: ["uuid1"]` — 按状态
- `assign_in: ["user_uuid"]` — 按负责人
- `project_in: ["project_uuid"]` — 按项目

## 时间戳

ONES 时间字段为**微秒级**，需 `÷ 1000000` 转秒级再格式化。

## 注意事项

- 所有 curl 加 `-s` 静默模式
- 每个 Bash 调用开头 `source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null`
- 推荐用 python3 urllib 处理 HTML 拼接和 JSON（避免 shell 转义问题）
