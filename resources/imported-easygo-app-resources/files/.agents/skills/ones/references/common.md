# ONES 公共说明

本 skill 面向 Standard Robots 的 ONES 实例。

## 常量

- `ONES_HOST`: `https://ones.standard-robots.com:10120`
- `API_BASE`: `${ONES_HOST}/project/api/project`
- `TEAM_UUID`: `UNrQ5Ny5`
- `GRAPHQL_ENDPOINT`: `${API_BASE}/team/${TEAM_UUID}/items/graphql`
- token 缓存文件：`~/.ones_token`

## 推荐工具

优先使用随 skill 提供的脚本：

- `python3 scripts/ones_api.py request GET /team/UNrQ5Ny5/task/149579/info`
- `python3 scripts/ones_api.py graphql '{ projects { uuid name } }'`
- `python3 scripts/ones_api.py login --print`
- `python3 scripts/ones_refresh_token.py`：刷新 `~/.ones_token`

只有在 helper 脚本不够用时，才写临时 `python3`。

## 认证

认证优先级（`ones_api.load_auth()` 实现）：

1. `ONES_USER_ID` + `ONES_AUTH_TOKEN`
2. `ONES_UUID` + `ONES_TOKEN`
3. `ONES_TOKEN_JSON`，格式为 `{"uuid": "...", "token": "..."}`（兼容 GitLab CI 变量）
4. `~/.ones_token`，格式为 `{"uuid": "...", "token": "..."}`
5. 401/403 时自动 `force_refresh`：
   - 有 `ONES_EMAIL` + `ONES_PASSWORD` → `POST /auth/login`
   - 否则 → 跑 `scripts/ones_refresh_token.py`（默认 `HEADLESS=1`）

刷新后会把 `{uuid, token}` 写回 `~/.ones_token`，权限自动设为 `600`。

### `~/.ones_token` 与浏览器 cookie 的关系

SR ONES 走飞书 OAuth 扫码登录，没有传统邮箱/密码可用。登录后
`.ones.standard-robots.com` 域下保留两枚 cookie：

| Cookie | 写入 `~/.ones_token` 字段 |
| --- | --- |
| `ones-uid` | `uuid` |
| `ones-lt` | `token`（64 字符） |

`scripts/ones_refresh_token.py` 用 Playwright 拉起 Chromium、复用持久化
profile `~/.cache/ones-chromium-profile` 完成上述提取。首次或飞书会话过期
时需要人工扫码，之后只要会话还在就能静默 `HEADLESS=1` 刷新。

注意：web 站点本身只用 cookie 鉴权（不带自定义 header），但
`/project/api/project/*` 接受外部 client 用 `Ones-User-Id` /
`Ones-Auth-Token` header 携带相同凭据。

通用请求头：

```text
Content-Type: application/json
Ones-Auth-Token: <token>
Ones-User-Id: <uuid>
Referer: https://ones.standard-robots.com:10120
```

## 默认负责人 / 问题解决人

**所有创建、流转、更新操作中涉及负责人（`owner`、`assign`）和"问题解决人"字段（`HiRJZNR7`，type=8）时，默认值必须是 ONES token 对应的用户 UUID（即 `~/.ones_token` 里的 `uuid`，等价于 `ones_api.load_auth()` 返回的 user_uuid）。**

理由：ONES 工单的工作量、解决数等统计按解决人归属。如果不是 token 用户，数据无法计入当前 AI 帐号。

适用规则：

- 创建工单（`tasks/add2`）：`owner` 与 `assign` 必须是 token 用户；缺陷/任务工单的 `HiRJZNR7` 字段默认填 token 用户。
- 状态 `transit` 后：transit 可能副作用变更 `assign`，必须紧跟一次 `tasks/update`，把 `assign`（缺陷/任务工单同时回写 `field_values` 中的 `HiRJZNR7`）显式恢复为 token 用户。
- 更新工单（`tasks/update`）：除非用户**明确**说"转给某某"或显式指定他人 UUID，否则不要修改 `assign` 和 `HiRJZNR7`，也不要拿其他人当默认值。
- 用户明确指定他人时按用户意图执行，不要自作主张覆盖。

`scripts/ones_create_task.py` 已经按此规则填 `owner` / `assign` / 用户型字段。手写 `tasks/add2` 或 `tasks/update` 请求时，需自己保证遵守。

## API 说明

### 获取工作项详情

`GET ${API_BASE}/team/UNrQ5Ny5/task/<ID>/info`

- 支持数字 ONES ID 或 task UUID
- 返回字段在顶层

### 通用更新

`POST ${API_BASE}/team/UNrQ5Ny5/tasks/update`

请求体：

```json
{"tasks": [{"uuid": "<task_uuid>", "desc_rich": "<html>", "field_values": []}]}
```

- 必须使用 task UUID
- `desc_rich` 是整段覆盖，不是追加
- 因此回写分析报告、处理结论或状态说明到描述/问题详情时，必须先读取当前 `desc_rich` 并在原 HTML 后追加新内容；禁止发送只包含 AI 报告的新 `desc_rich` 覆盖原问题详情
- 如果无法获取或可靠保留原 `desc_rich`，改用评论接口 `send_message` 追加说明，不要调用 `tasks/update` 修改 `desc_rich`

### 评论

- 查看：`GET /team/UNrQ5Ny5/task/<task_uuid>/messages`
- 新增：`POST /team/UNrQ5Ny5/task/<task_uuid>/send_message`

评论请求体：

```json
{"uuid": "<8-char random>", "text": "<comment>"}
```

### 状态流转

- 查询可用流转：`GET /team/UNrQ5Ny5/task/<task_uuid>/transitions`
- 触发流转：`POST /team/UNrQ5Ny5/task/<task_uuid>/transit`

工作流状态不要只更新 `status_uuid`，要走 transit API。

### 附件

- 列表：`GET /team/UNrQ5Ny5/task/<task_uuid>/attachments?count=100`
- 批量获取签名下载链接：`GET /team/UNrQ5Ny5/res/attachments?ids=<uuid1>,<uuid2>,...`

优先用批量接口，单个附件接口偶尔会出现瞬时 404。

### 辅助字典数据

- Members: `GET /team/UNrQ5Ny5/members?limit=500`
- Issue types: `GET /team/UNrQ5Ny5/issue_types`
- Statuses: `GET /team/UNrQ5Ny5/task_statuses`
- Fields: `GET /team/UNrQ5Ny5/fields?pool=task`

### GraphQL

项目列表：

```graphql
{ projects { uuid name } }
```

项目内工作项类型：

```graphql
{ project(uuid: "<project_uuid>") { issueTypes { uuid name } } }
```

工作项列表：

```graphql
{
  tasks(filter: {...}, limit: N, orderBy: {createTime: DESC}) {
    uuid
    name
    number
    createTime
    issueType { uuid name }
    status { uuid name }
    assign { uuid name }
  }
}
```

迭代列表：

```graphql
{
  sprints(
    filter: { project_in: ["<project_uuid>"] }
    limit: 10
    orderBy: { createTime: DESC }
  ) {
    uuid
    name
  }
}
```

## 附件处理

- `image/*`：下载到 `/tmp/ones-analyze/<ID>/images/`
- `.log.tar.zst`：`zstd -d` 后再 `tar xf`
- `.tar` 且内部有 `.gz`：先 `tar xf` 再 `gunzip`
- `.zip`：`unzip -o`
- 非日志且超过 50 MB 的文件：只列信息，不自动下载

## AI 标记

评论前缀：

- `[AI Agent] 正在处理中`
- `[AI Agent] 处理完成`
- `[AI Agent] 处理失败`

HTML 报告声明：

```html
<div style="background:#fff3cd;border:1px solid #ffc107;padding:12px;margin-bottom:16px;border-radius:4px">
<p style="margin:0"><b>声明：本报告由 AI 辅助分析生成，仅供参考。</b></p>
</div>
```

## 已处理判断

满足以下任一条件即可认为该工作项已被 AI 处理：

- discussion 评论里含 `[AI Agent] 处理完成`
- discussion 评论里含 `[AI Agent] 正在处理中`
- `desc_rich` 含 `本报告由 AI 辅助分析生成`
- `desc_rich` 同时含 `分析报告` 和 `根因`

## 类型到流程的映射

| 分类 | 工作项类型 | UUID | 推荐流程 |
| --- | --- | --- | --- |
| 问题分析 | 现场问题 | `ARCxvBDk` | analyze |
| 问题分析 | 现场缺陷 | `VGyUpCRi` | analyze |
| 问题分析 | 客户端问题库 | `Xc7sF3tN` | analyze |
| 问题分析 | 产品研发部问题库 | `CeENe67t` | analyze |
| 问题分析 | 疑难问题库 | `2ugm5dXZ` | analyze |
| 问题分析 | 研发端问题库 | `NPGuDBtg` | analyze |
| 缺陷修复 | 软件缺陷 | `NuSzzA62` | bugfix |
| 缺陷修复 | 缺陷 | `8YtVcwaj` | bugfix |
| 需求实现 | 软件产品需求 | `GbtzHVF6` | implement |
| 需求实现 | 需求 | `Roy6EvKA` | implement |

## 时间

ONES 时间戳是微秒，格式化前先除以 `1000000`。
