# 创建

当用户要新建 ONES 的需求、缺陷或任务时，用这个流程。

## 推荐路径

优先使用随 skill 提供的 helper：

```bash
python3 scripts/ones_create_task.py "清理测试中的opencv依赖" --type task --repo-root /workspace/sros-core
```

常用参数：

- `--type requirement|defect|task`
- `--project <name>` 或 `--project-uuid <uuid>`
- `--priority p0|p1|p2|p3`
- `--desc "<rich text source>"`
- `--json`

## 项目选择

优先级：

1. 明确指定 project UUID
2. 明确指定 project name
3. 请求里引用了现有工单时，继承它的项目
4. 默认项目 `SROS开发`，UUID 为 `2ni3183ALTfZNzBu`

如果项目匹配不唯一，先问一个简短问题再创建。

## 类型选择

`SROS开发` 的默认类型映射：

- requirement -> `Roy6EvKA`
- defect -> `8YtVcwaj`（SROS开发 项目下的「缺陷」；不要用「软件缺陷」`NuSzzA62`，该类型不在本项目 IssueTypeScope，会报 `NotFound.IssueTypeScope`）
- task -> `B7aTPV3N`

> **defect 创建已知限制**：SROS开发 项目的「缺陷」类型还要求 `publish_version` 等系统字段（不是 UUID 形式的自定义字段，本脚本的 `MissingParameter` 重试逻辑只能处理自定义字段），可能仍报 `MissingParameter.Task.publish_version` / `MissingParameter.Task.PMDVKAdZ` 等。脚本场景请优先 `--type task`；要建真缺陷请用 ONES web UI，或扩展脚本识别系统字段名。

其他项目要先查询 issue types，再按名称模糊匹配。

## 迭代必填

这里继承 `jialipeng/improve-ones-create-skill` 的改动：

1. 从仓库 `CMakeLists.txt` 读取 `SROS_MAJOR_VERSION`、`SROS_MINOR_VERSION`、`SROS_PATCH_VERSION`
2. 拼成 `v{major}.{minor}.{patch}`，例如 `v5.68.0`
3. 查询目标项目最近的 sprints
4. 选择名称与当前版本匹配的 sprint
5. 如果匹配不到，兜底到需求池 sprint `XCB4dHYh`

任何类型都不要把 `sprint_uuid` 留空，否则 ONES 会返回 `NotFound.Sprint`。

## 必填字段

### 需求

`SROS开发` 的需求默认补这些字段：

- `aprna2v3` 变更类型：新增 `4brebLj8`，优化 `PTiiHzYC`，变更 `SsMwidaX`
- `65LHQvNj` 产品经理：ONES token 对应用户 UUID（详见 common.md「默认负责人 / 问题解决人」）
- `416pdiVz` 产品系列：`Qxr2oWw5`
- `4ytkWp5h` 关联调度：`VqBy8wyP`
- `LTN9JJqe` 项目名称：`无`
- `VvbkFLvF` 信息扩散度：`qUJCe1iR`
- `PkScSNWa` 需求来源：`5cW9RBT7`
- `Epka4E6A` 需求类型：新功能 `274UHSTA`，优化 `B1suru9P`，文档 `QYjsWUda`
- `AJc4frqJ` 需求说明：由请求文本自动生成
- `SgUUtEB6` 需求种类：`8eDQQRhu`
- `F1BfcE3e` 需求类别：字段定义里的第一个可用选项

### 任务和缺陷

这里继承另一处改动：

- `HiRJZNR7` 问题解决人：ONES token 对应用户 UUID（详见 common.md「默认负责人 / 问题解决人」）

并保留自动重试逻辑：

- 如果 create 返回 `MissingParameter.Task.<field_uuid>`，就去查询 task 字段定义
- 如果字段是用户字段（`type=8`），填 ONES token 用户 UUID
- 否则优先用 `default_value`，再退到第一个可用选项
- 有界重试几次，不无限循环

始终传 `parent_uuid: ""`，避免 `MissingParameter.Task.ParentTask`。

## 创建接口

`POST /team/UNrQ5Ny5/tasks/add2`

请求体结构：

```json
{
  "tasks": [{
    "uuid": "<creator-prefix(取 token 用户) + 8 random chars>",
    "owner": "<token_user_uuid>",
    "assign": "<token_user_uuid>",
    "summary": "<title>",
    "parent_uuid": "",
    "sprint_uuid": "<resolved sprint>",
    "project_uuid": "<project_uuid>",
    "issue_type_uuid": "<issue_type_uuid>",
    "desc_rich": "<html>",
    "priority": "<priority_uuid>",
    "field_values": [...]
  }]
}
```

## 返回结果

返回创建后的编号、标题、项目、类型和直达链接。
