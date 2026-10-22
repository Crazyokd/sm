# ONES 发布管理 (release)

ONES 里"发布"不是独立 entity, 而是 `task` 类型, 用 `issueTypeScope_uuid` 区分。
URL `component/4g7onaeb/view/5qrfZ4x5/task/<uuid>` 里的 `task/<uuid>` 就是 release task。

## 关键 UUID (SROS 项目)

| 名称 | UUID | 说明 |
| --- | --- | --- |
| 项目 SROS开发 | `2ni3183ALTfZNzBu` | 项目级过滤用 |
| 发布工作项 issueTypeScope | `XC4uLZFY` | `tasks(filter: {issueTypeScope_in: [...]})` 限定为发布类型 |
| team | `UNrQ5Ny5` | 所有 ONES API 路径前缀 |

## 优先使用脚本

```bash
# 默认 = SROS + 未发布 + 标题含"必须完成项" (autopilot 入口)
python3 scripts/ones_list_releases.py

# 展开关联清单 (publish_content), 每条发布额外多调一次 ONES API
python3 scripts/ones_list_releases.py --include-content

# 自定义状态/关键字
python3 scripts/ones_list_releases.py --status "已发布" --name-contains "5.66"

# 已知 release uuid 时直接拿 publish_content, 跳过列表查询
python3 scripts/ones_list_releases.py --release-uuid Uea8vXdNNvZUAc97 --include-content
```

退出码: `0` 成功 / `1` 参数错 / `2` ONES 调用失败。

## 已知陷阱: root activityReleases 不可用

| 想做的 | 结论 |
| --- | --- |
| `{ activityReleases }` 列所有发布 | ✗ 恒返回 5 条全局发布, 任何分页/过滤参数都被忽略 |
| `{ project(key: "...") { activityReleases } }` | ✗ Project type 不暴露 activityReleases 子字段 |
| `{ activityRelease(key: "<release-task-uuid>") }` | ✗ 该 key 对应的是 task, 不是 ActivityRelease, 返 500 |
| **`{ tasks(filter: {project_in, issueTypeScope_in}) }`** | ✓ 唯一可行路径, 见上 |

## 关联清单 publish_content

每个 release task 的 `info` 返回包含:

- `publish_content_count` — 关联工作项总数
- `publish_content_done_count` — 已完成数
- `publish_content` — 关联工作项数组, 每条是完整 task 对象 (uuid / number / summary / `issue_type_uuid` / `status_uuid` / owner / assign / project_uuid / ...)

实测 `#148851 [待发布]SROS_5.66.x 必须完成项` → `publish_content_count = 190` (158 done)。

二次过滤 (客户端):

- 缺陷类型: 把 `publish_content[i].issue_type_uuid` 比对类型字典 (软件缺陷 `NuSzzA62` / 现场缺陷 `VGyUpCRi` / 其它见 references/common.md 的"类型到流程的映射")
- "分析与解决"状态: 把 `publish_content[i].status_uuid` 比对 task_statuses 字典 (调 `/team/UNrQ5Ny5/task_statuses` 查映射)

## GraphQL 原始查询 (脚本背后)

```graphql
{
  tasks(
    filter: { project_in: ["2ni3183ALTfZNzBu"], issueTypeScope_in: ["XC4uLZFY"] }
    limit: 5000
  ) {
    uuid
    number
    name
    status { uuid name }
    issueType { uuid name }
  }
}
```

注意:

- `name` 字段 (GraphQL) 等价于 REST `info` 的 `summary` 字段, 不要混用
- 字段名拼错或 `orderBy` 不被支持时 ONES 不报错, 直接返回 `[]` — 写新查询前先用 `--compact` 单跑确认
- `tasks` 的 `limit` 默认很小, 整项目 task 数轻松过万, 列表查询写不到 limit 会丢数据
- filter operator: `_in` / `_eq` / `_range` 合法; `_like` / `_contains` / `_starts_with` 不合法; 字符串模糊匹配要客户端做

## 何时**不**走本脚本

- 单条已知 task UUID 查详情 → 用 `python3 scripts/ones_api.py request GET /team/UNrQ5Ny5/task/<uuid>/info`, 直接读 `publish_content` 字段
- 不是"发布"类型的 task 检索 → 用 `tasks(filter: {project_in, issueType_in})`, 见 references/view.md
