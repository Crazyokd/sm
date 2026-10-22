---
description: 'ONES 工作项一站式处理：查看、分析、修复、创建、更新、评论'
allowed-tools: Bash(curl, source, cat, mkdir, tar, zstd, gunzip, unzip, find, ls, head, tail, wc, python3, echo, file, du, sed, awk, grep, md5sum, zcat), Read(**), Glob(**), Grep(**), Agent, AskUserQuestion
argument-hint: '<工作项ID|自然语言指令>'
---

# ONES 工作项处理

统一入口，根据用户意图自动路由到对应功能。

## 参数

$ARGUMENTS

## 意图识别与路由

根据用户输入**自动判断**执行哪个功能：

| 用户输入示例 | 识别为 | 执行 |
|-------------|--------|------|
| `148707` | 查看 | 工作项详情 → `ones/view.md` 模式 A |
| `148707 --raw` | 查看(原始) | 输出原始 JSON |
| `最近3天的现场问题` | 列表查询 | 筛选+表格展示 → `ones/view.md` 模式 B |
| `148234 148235` | 批量查看 | 多工作项列表 → `ones/view.md` 模式 B |
| `分析 148707` | 分析 | 双Agent独立分析+报告 → `ones/analyze.md` |
| `修复 148707` | 修复 | 分析→代码修复→MR → `ones/bugfix.md` |
| `实现 148742` | 实现 | 需求→方案→代码→MR → `ones/implement.md` |
| `创建 需要支持xxx` / `创建缺陷 xxx` | 创建 | 创建工作项 → `ones/create.md` |
| `更新 148707 状态为已解决` / `转给张三` | 更新 | 修改字段 → `ones/update.md` |
| `评论 148707` / `评论 148707 已修复` | 评论 | 查看或添加 → `ones/comment.md` |
| `关闭 148707` | 更新 | 状态转为已关闭 → `ones/update.md` |
| `分析 148234 148235` | 批量处理 | 逐个调用对应 skill（见下方批量规则） |

**识别规则**：
- 纯数字/UUID → **查看**
- 含"分析/analyze/排查" → **分析**
- 含"修复/fix/bugfix" → **修复**
- 含"实现/implement/开发/编码" → **实现**
- 含"创建/create/新建/提需求/提缺陷" → **创建**
- 含"更新/update/修改/转给/关闭/完成/变更" → **更新**
- 含"评论/comment/留言/备注" → **评论**
- 含筛选词（"最近/批量/所有/未处理"）→ **列表查询**（view 模式 B）
- 多个 ID + 动作词 → **批量处理**
- 多个 ID 无动作词 → **批量查看**（view 模式 B）
- 无法判断时用 AskUserQuestion 确认

## 批量处理规则

当输入包含多个 ID 和动作词（分析/修复/实现）时，逐个调用对应 skill：

1. 先用 `ones/view.md` 模式 B 获取所有工作项信息并展示列表
2. 确认用户要处理哪些（默认全部未处理的）
3. 逐个串行调用对应 skill（≤2 个时可并行）
4. 汇总结果

> 如果无动作词，根据工作项类型自动匹配处理方式（参见 `common.md` 类型→处理方式映射表）。

## 各功能详细定义

见 `ones/` 子目录下的对应文件：
- `ones/common.md` — 公共基础（认证、API、类型映射、已处理检测）
- `ones/view.md` — 查看（单个详情 + 列表筛选）
- `ones/analyze.md` — 分析
- `ones/bugfix.md` — 修复（分析→修复→MR）
- `ones/implement.md` — 实现（需求→方案→代码→MR）
- `ones/create.md` — 创建（需求/缺陷/任务）
- `ones/update.md` — 更新（状态、负责人、优先级）
- `ones/comment.md` — 评论（查看/添加）
