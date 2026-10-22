# ONES 查看工作项

支持两种模式：**单个详情查看** 和 **列表筛选查询**。

## 参数

- `<工作项ID>`：序号（如 148707）或 UUID → 单个详情
- `<多个ID>`：如 `148234 148235` → 批量详情
- `<自然语言筛选>`：如"最近3天的现场问题" → 列表查询
- `--raw`：输出原始 JSON

## 前置依赖

认证流程见 `common.md`。

---

## 模式 A：单个详情查看

当输入为单个数字 ID 或 UUID 时执行。

### Phase 1：认证

按 `common.md` 执行。

### Phase 2：并行获取数据

以下 API **全部并行执行**（多个 Bash tool call）：

| # | API | 用途 |
|---|-----|------|
| a | `GET /team/UNrQ5Ny5/task/<ID>/info` | 工作项详情 |
| b | `GET /team/UNrQ5Ny5/members?limit=500` | 成员列表（UUID→姓名） |
| c | `GET /team/UNrQ5Ny5/issue_types` | 类型列表（UUID→类型名） |
| d | `GET /team/UNrQ5Ny5/task_statuses` | 状态列表（UUID→状态名） |
| e | `GET /team/UNrQ5Ny5/fields?pool=task` | 字段定义（优先级 field012→选项名） |
| f | `GET /team/UNrQ5Ny5/project/<project_uuid>/sprints` | 迭代列表（需 project_uuid，来自 a） |
| g | `GET /team/UNrQ5Ny5/task/<task_uuid>/attachments?count=100` | 附件列表（需 task_uuid，来自 a） |

> f 和 g 依赖 a 的返回值。如果用户传入序号，先等 a 返回再发 f/g。

### Phase 3：UUID 解析

构建映射表：

| 原始字段 | 查找来源 | 匹配方式 |
|----------|----------|---------|
| `owner` | 成员列表 | uuid → name |
| `assign` | 成员列表 | uuid → name |
| `issue_type_uuid` | 类型列表 | uuid → name |
| `status_uuid` | 状态列表 | uuid → name |
| `priority` | 字段定义 field012 | uuid → value |
| `sprint_uuid` | 迭代列表 | uuid → title |

无法解析时显示原始 UUID。

### Phase 4：格式化输出

`--raw` 时直接输出 JSON。否则：

```
## [类型名] #序号：标题

| 属性 | 值 |
|------|-----|
| 类型 | ... |
| 状态 | ... |
| 优先级 | ... |
| 负责人 | ... |
| 创建者 | ... |
| 创建时间 | 格式化（微秒÷1000000） |
| 所属项目 | project_name |
| 所属迭代 | sprint title |

### 描述
<desc_rich 转 Markdown>

### 附件
| # | 文件名 | 类型 | 大小 |
|---|--------|------|------|
```

### Phase 5：附件处理

- **图片**（image/*）：下载到 `/tmp/ones/<ID>/`，Read 工具展示
- **小文件**（<10MB 文本/日志）：自动下载展示关键内容
- **压缩日志**：提供下载命令
- **大文件**（>50MB）：仅列出信息

> 下载前 `mkdir -p /tmp/ones/<工作项ID>/`

---

## 模式 B：列表筛选查询

当输入为自然语言筛选条件或多个 ID 时执行。

**示例**：
```
/ones 最近3天的现场问题，取5条
/ones 软件缺陷，状态是分析与解决
/ones 分配给张三的未关闭缺陷
/ones 148234 148235 148236
```

### Phase 1：解析筛选条件

**自然语言模式**：从用户描述提取筛选维度，构造 GraphQL 查询（参见 `common.md` GraphQL 列表查询）。

| 维度 | GraphQL 参数 | 示例 |
|------|-------------|------|
| 工作项类型 | `issueType_in` | "现场问题" → `["ARCxvBDk"]`（参见 `common.md` 类型映射表） |
| 时间范围 | `createTime_range: {from, to}` | "最近3天" → 计算微秒时间戳 |
| 状态 | `status_in` | "分析与解决" → 需先查状态 UUID |
| 负责人 | `assign_in` | "张三" → 需先查成员 UUID |
| 数量 | `limit` | "取5条" → 5，默认 10 |

**多 ID 模式**：并行 `GET /team/UNrQ5Ny5/task/<ID>/info`。

### Phase 2：检查已处理状态

对每个工作项检查 AI 处理状态（见 `common.md` 已处理检测规则）。

### Phase 3：展示列表

```
## 查询到 N 个工作项

| # | ID | 类型 | 标题 | 状态 | 负责人 | AI处理 |
|---|-----|------|------|------|--------|--------|
| 1 | 148426 | 现场问题 | xxx | 分析与解决 | 张三 | 未处理 |
| 2 | 148373 | 现场问题 | xxx | 已关闭 | 李四 | 已处理 |
```
