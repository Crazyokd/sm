# ONES 创建工作项

解析用户描述，推断类型和字段，创建工作项（需求、缺陷或任务）。

## 参数

$ARGUMENTS

- `--no-confirm`：跳过确认直接创建
- `--project <项目名>`：指定目标项目（默认 SROS开发）
- `--type <类型>`：强制指定类型（需求/缺陷/任务）

## 前置依赖

认证流程见 `common.md`。

---

## 执行流程

### Phase 1：认证

按 `common.md` 执行。

### Phase 2：确定项目和类型

#### 项目选择（优先级从高到低）

1. **`--project` 参数**：用户明确指定项目名
2. **关联工作项推断**：如果描述中提到工作项 ID（如"关联 148707"），获取该工作项所属项目并继承
3. **默认项目**：SROS开发（`2ni3183ALTfZNzBu`）
4. **无法推断**：查询项目列表（GraphQL `{ projects { uuid name } }`），AskUserQuestion 让用户选择

> 项目名模糊匹配（包含即匹配）。多个匹配时列出让用户选择。

#### 类型推断

| 关键词 | 类型 | 默认 UUID |
|--------|------|-----------|
| 缺陷/bug/defect/修复 | 软件缺陷 | `NuSzzA62` |
| 任务/task/todo/办 | 任务 | `B7aTPV3N` |
| （默认）需求/feature/需要/支持 | 需求 | `Roy6EvKA` |

> UUID 为 SROS开发 项目的常用类型。其他项目的类型 UUID 可能不同，需动态查询：`GET /team/UNrQ5Ny5/issue_types`，按名称匹配。

### Phase 3：推断字段

#### 迭代（sprint_uuid）— 必填

默认填写**当前迭代**，推断方式：

1. 从 `CMakeLists.txt` 读取版本号：`SROS_MAJOR_VERSION`、`SROS_MINOR_VERSION`、`SROS_PATCH_VERSION`，拼成 `v{major}.{minor}.{patch}`（如 `v5.68.0`）
2. 通过 GraphQL 查询迭代列表，按名称匹配：
```
POST ${GRAPHQL_ENDPOINT}
Body: {"query": "{ sprints(filter: { project_in: [\"<project_uuid>\"] }, limit: 10, orderBy: { createTime: DESC }) { uuid name } }"}
```
3. 找到名称匹配的迭代 UUID 填入 `sprint_uuid`
4. 若匹配失败，使用需求池（`XCB4dHYh`）兜底

> **所有工作项类型（需求、缺陷、任务）都必须填写迭代**，不可留空，否则 API 会返回 `NotFound.Sprint` 错误。

#### 标题（summary）
从描述中提取简洁标题（≤1024 字符）。

#### 优先级（priority）

| 关键词 | 级别 | UUID |
|--------|------|------|
| 紧急/严重/P0 | 最高 | `N3f3Lpa7` |
| 重要/P1 | 较高 | `X4zCjKmD` |
| （默认） | 普通 | `4Fox75xJ` |
| 低优/P3 | 较低 | `7uf9Egvm` |

#### 必填自定义字段（SROS开发 + 需求）

| 字段名 | field_uuid | 推断逻辑 |
|--------|-----------|---------|
| 变更类型 | `aprna2v3` | 新增→`4brebLj8` / 优化→`PTiiHzYC` / 变更→`SsMwidaX` |
| 产品经理 | `65LHQvNj` | ONES token 对应用户 UUID |
| 产品系列 | `416pdiVz` | All（`Qxr2oWw5`） |
| 关联调度 | `4ytkWp5h` | 通用（`VqBy8wyP`） |
| 项目名称 | `LTN9JJqe` | `"无"` |
| 信息扩散度 | `VvbkFLvF` | 内部（`qUJCe1iR`） |
| 需求来源 | `PkScSNWa` | 开发人员（`5cW9RBT7`） |
| 需求类型 | `Epka4E6A` | 新功能→`274UHSTA` / 优化→`B1suru9P` / 文档→`QYjsWUda` |
| 需求说明 | `AJc4frqJ` | 自动生成 |
| 需求种类 | `SgUUtEB6` | 技术需求（`8eDQQRhu`） |
| 需求类别 | `F1BfcE3e` | 默认第一个选项 |

#### 必填自定义字段（SROS开发 + 任务/缺陷）

| 字段名 | field_uuid | 类型 | 推断逻辑 |
|--------|-----------|------|---------|
| 问题解决人 | `HiRJZNR7` | 8（用户） | 默认填 ONES token 对应用户 UUID（详见 common.md「默认负责人 / 问题解决人」） |

> **缺陷和任务的必填字段不同**：创建时如遇 `MissingParameter.Task.<field_uuid>` 错误，自动查字段定义（`GET /fields?pool=task`）→ 找到该字段的第一个选项或填 token 用户（用户类型字段 type=8）→ 重试（最多 5 次）。

### Phase 4：确认（除非 --no-confirm）

展示推断结果表格，AskUserQuestion 确认。

### Phase 5：创建

#### UUID 生成
`创建者UUID(8位) + 随机8位` — ONES 强制规则。创建者 UUID 取 ONES token 用户。

#### owner / assign

`owner` 与 `assign` 必须填 ONES token 用户 UUID（不要填其他成员），否则统计无法归属当前帐号。详见 common.md「默认负责人 / 问题解决人」。

#### API 调用
```
POST /team/UNrQ5Ny5/tasks/add2
Body: {"tasks": [{ uuid, owner, assign, summary, parent_uuid:"",
       sprint_uuid, project_uuid, issue_type_uuid, desc_rich, priority,
       field_values: [...] }]}
```

**迭代**：按 Phase 3 推断的 `sprint_uuid` 填入（从 CMakeLists.txt 版本号匹配当前迭代）。

#### 错误恢复
`MissingParameter.Task.<field_uuid>` → 查字段定义 → 填默认值 → 重试（最多 5 次）。

### Phase 6：输出结果

```
已创建 [类型] #<序号>：<标题>
项目：<项目名>
链接：https://ones.standard-robots.com:10120/project/#/team/UNrQ5Ny5/task/<uuid>
```
