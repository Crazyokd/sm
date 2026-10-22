# 查看与列表

当用户想查看 ONES 工作项时，用这个流程。

## 输入类型

- 单个数字 ID 或 task UUID
- 多个 ID
- 像 `最近3天的现场问题` 这样的自然语言筛选
- 类似 `--raw` 的原始 JSON 输出需求

## 单个工作项

1. 先读 [common.md](./common.md)。
2. 先拿 task info：

   ```bash
   python3 scripts/ones_api.py request GET /team/UNrQ5Ny5/task/<ID>/info
   ```

3. 并行获取需要的字典数据：
   - members
   - issue types
   - statuses
   - task fields
   - 当前项目的 sprints
   - 当前 task UUID 的 attachments
4. 把 owner、assign、issue type、status、priority、sprint 的 UUID 解析成名字。
5. 输出：
   - 原始 JSON，或
   - 简洁摘要表加描述和附件

## 多个 ID

1. 并行获取每个 task info。
2. 解析类型、状态、负责人和 AI 处理状态。
3. 用表格展示编号、类型、标题、状态、负责人、AI 状态。

## 自然语言筛选

把用户描述翻译成 GraphQL filter：

- 类型 -> `issueType_in`
- 时间范围 -> `createTime_range`
- 状态 -> `status_in`
- 负责人 -> `assign_in`
- 项目 -> `project_in`
- 数量 -> `limit`

然后查询 GraphQL 并展示表格。

## 附件规则

- 图片：需要时下载并查看
- 小文本日志：下载并做摘要
- 大文件或纯二进制：只列信息，不要直接倾倒内容
- 下载前先创建本地目录

## 输出形态

优先保持简洁：

```text
## [类型] #148707：标题

类型: ...
状态: ...
优先级: ...
负责人: ...
创建者: ...
创建时间: ...
项目: ...
迭代: ...

描述:
...

附件:
- xxx.log.tar.zst
- screenshot.png
```
