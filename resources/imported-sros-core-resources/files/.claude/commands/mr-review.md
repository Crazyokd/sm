---
description: 'GitLab MR 代码审查：批量审查所有开放 MR，发表行内评论'
allowed-tools: Bash(curl, git, python3, source, echo, cat, head, tail, wc, ls, find, grep, sed, awk, mkdir, rm, date), Read(**), Glob(**), Grep(**), Agent, AskUserQuestion
argument-hint: '[MR_IID] [--dry-run]'
---

# GitLab MR 代码审查

批量审查 SROS 项目所有开放的 GitLab Merge Request，在对应位置发表行内评论。

## 参数

$ARGUMENTS

- 无参数：审查所有开放 MR
- `<IID>`：只审查指定 MR（如 `/review 11989`）
- `--dry-run`：分析但不发表评论，仅输出到控制台

---

## 常量

- **GITLAB_URL**: `http://git.standard-robots.com`
- **PROJECT_ID**: `38`
- **API_BASE**: `$GITLAB_URL/api/v4/projects/$PROJECT_ID`
- **Auth Header**: `PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN`
- **Bot 隐藏标记**: `<!-- [Code Review Bot] -->`（用于机器识别自己的评论）
- **Bot 可见签名**: `🤖 AI Review`（所有评论末尾必须带此标识，让读者明确知道这是 AI 生成的评审，非真人评论）

---

## Phase 0: 认证与参数解析

### 0.1 验证 Token

```bash
source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null
```

检查 `GITLAB_PRIVATE_TOKEN` 是否存在。若为空，停止并提示：
```
请先设置环境变量：export GITLAB_PRIVATE_TOKEN="your-token"
可从 GitLab 个人页面 Access Tokens 获取
```

### 0.2 解析参数

从 `$ARGUMENTS` 中提取：
- 是否包含数字 → 单个 MR IID
- 是否包含 `--dry-run` → dry-run 模式

---

## Phase 1: 列出并过滤 MR

### 1.1 获取所有开放 MR

```bash
curl -s "$API_BASE/merge_requests?state=opened&per_page=100" \
  -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"
```

如果 `$ARGUMENTS` 指定了 IID，改为：
```bash
curl -s "$API_BASE/merge_requests/$IID" \
  -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"
```

### 1.2 过滤不需要审查的 MR

用 python3 过滤掉以下 MR：
- `state == "merged"` 或 `state == "closed"` — 已合并/已关闭
- `work_in_progress == true` 或 `draft == true` — 草稿
- `title` 以 `Draft:` 或 `WIP:` 开头（不区分大小写）

指定 IID 时也需检查：若该 MR 已合并或已关闭，直接提示并跳过。

额外跳过规则（变更内容过滤）：
- 若 MR 只修改了 `conan/arch_lib/conanfile.txt` 或 `conan/public_lib/conanfile.txt`（Conan 依赖版本变更），**跳过**，打印 "仅 Conan 依赖变更，交给人工审查"

### 1.3 输出汇总表

显示 MR 列表表格：IID、标题、作者、源分支 → 目标分支、流水线状态。标记哪些将被审查、跳过。

---

## Phase 2: 逐 MR 分流

对每个非 Draft MR **依次**执行以下检查。每处理完一个 MR 打印一行状态。

### 2.1 Cherry-pick 检测

检查 `source_branch` 是否匹配 `cherry-pick-*` 模式。

若为 cherry-pick MR：
- 获取流水线状态：`GET $API_BASE/merge_requests/{iid}/pipelines`
- 最新流水线 `status == "success"` →
  - 批准：`POST $API_BASE/merge_requests/{iid}/approve`
  - 发一条评论：`<!-- [Code Review Bot] -->\nCherry-pick MR，CI 已通过，自动批准。\n\n---\n🤖 *AI Review — 仅供参考，非真人评论*`
  - **跳过**，处理下一个
- 否则：**跳过**，打印 "Cherry-pick MR，CI 未通过"

### 2.2 CI 流水线检查

```bash
curl -s "$API_BASE/merge_requests/{iid}/pipelines" \
  -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"
```

取第一条（最新）流水线：

| 状态 | 操作 |
|------|------|
| `success` | 继续审查 |
| `failed` | 重试失败的 job（见下方详细步骤），然后**跳过**此 MR（下次运行时再审查） |
| `running` / `pending` / `created` | **跳过**，打印 "CI 运行中" |
| 无流水线 | **跳过**，打印 "无 CI 流水线" |

**CI 失败重试详细步骤**：
1. 获取该 pipeline 的所有 job：`GET $API_BASE/pipelines/{pipeline_id}/jobs?per_page=100`
2. 筛选 `status == "failed"` 的 job
3. 打印失败 job 信息：`!{iid} Pipeline {pipeline_id} — 失败 Job: {job_name} (stage: {stage})`
4. 逐个重试失败的 job：`POST $API_BASE/jobs/{job_id}/retry`（注意是 job 级别，不是 pipeline 级别）
5. 重试后不等待结果，直接处理下一个 MR

### 2.3 已审查检查

并行获取讨论和提交：

```bash
# 获取讨论
curl -s "$API_BASE/merge_requests/{iid}/discussions" \
  -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"

# 获取提交
curl -s "$API_BASE/merge_requests/{iid}/commits" \
  -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"
```

用 python3 判断：
1. 在 discussions 中查找包含 `<!-- [Code Review Bot] -->` 的 note
2. 找到最新的 bot 评论时间
3. 比较 commits 中最新提交的 `created_at` 与 bot 最后评论时间
4. 若 bot 已评论 且 作者无新提交 → **跳过**，打印 "已审查，无新提交"
5. 若 bot 已评论 但 作者有新提交 → 继续审查（增量审查）
6. **同时提取人工评论**：收集 discussions 中非 bot（不含 `<!-- [Code Review Bot] -->`）的评论，提取 `author.name`、`body`（截取前 300 字符），作为上下文传给 Agent

### 2.4 获取 MR 变更

```bash
curl -s "$API_BASE/merge_requests/{iid}/changes?access_raw_diffs=true" \
  -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"
```

提取：
- `diff_refs`（base_sha, head_sha, start_sha）— 发表行内评论必需
- `changes` 数组 — 每个文件的 diff
- 保存 existing discussions 用于去重

同时执行 `git fetch origin {source_branch}` 以便 Agent 可以通过 `git show` 读取完整源文件。

### 2.5 获取 ONES 工作项上下文

从 MR 标题中提取所有 6 位数字的 ONES 工单号（正则 `#(\d{6})` 全局匹配，可能有多个）。标题中可能关联多个 ONES 工单，如 `#149063#149079 fix(usart): ...` 或 `#149063 #149079 fix: ...`。

对每个提取到的 ONES ID，调用 ONES API 获取工作项详情：

```bash
source ~/.zshrc 2>/dev/null || source ~/.bashrc 2>/dev/null
ONES_TOKEN=$(python3 -c "import json; d=json.load(open('$HOME/.ones_token')); print(d['token'])")
ONES_UUID=$(python3 -c "import json; d=json.load(open('$HOME/.ones_token')); print(d['uuid'])")

# 对每个 ONES_ID 调用
curl -s "https://ones.standard-robots.com:10120/project/api/project/team/UNrQ5Ny5/task/${ONES_ID}/info" \
  -H "Content-Type: application/json" \
  -H "Ones-Auth-Token: $ONES_TOKEN" \
  -H "Ones-User-Id: $ONES_UUID" \
  -H "Referer: https://ones.standard-robots.com:10120"
```

从每个返回中提取关键字段（用 python3）：
- `summary`：工作项标题（注意：不是 `name`，ONES API 用 `summary` 表示标题）
- `desc_rich` 或 `desc`：需求/问题描述（HTML 转纯文本，截取前 500 字符避免过长）
- `number`：工作项编号

注意：`issue_type_uuid`、`status_uuid`、`priority` 等字段返回的是 UUID 而非名称，无需解析。

将所有工作项上下文汇总后传给 Agent，格式如：
```
ONES #149063: 串口调试信息不足，无法定位通信问题
  描述：客户反馈串口日志缺少关键帧数据...
ONES #149079: AGV-600E Matrix SSH 连接异常
  描述：现场设备 SSH 连接超时...
```

若某个 ONES ID 查询失败（401/404/网络错误），跳过该条并打印警告，**不阻塞审查流程**。

---

## Phase 3: Agent 审查 + 即时发表评论

对每个合格 MR，**依次**执行：Agent 审查 → 去重 → 发表评论 → 发表总结。**审完一个 MR 就立即提交评论到 GitLab**，不要等所有 MR 全部审完再批量提交。

### Agent 输入

提供以下信息给 Agent：

1. **MR 元数据**：IID、标题、作者、source_branch、target_branch、描述（description 可能包含测试结果、截图链接等有价值信息）
2. **完整 diff**：所有变更文件的 diff 内容
3. **已有 bot 评论**：用于避免重复
4. **源分支名**：Agent 可使用 `git show origin/{source_branch}:{path}` 读取完整文件上下文
5. **ONES 工作项上下文**（若获取成功）：所有关联工作项的标题、类型、描述摘要。帮助 Agent 理解改动的需求来源和问题背景，一个 MR 可能关联多个工作项
6. **人工评论上下文**（若存在）：其他开发者在 MR 上的评论，可能包含真机测试结果、领域经验、设计讨论等有价值信息，帮助 Agent 更好理解改动背景和已知问题

### Agent 审查指令

```
你是 SROS 机器人项目的代码审查专家。请严格按以下规则审查此 MR，用中文输出。

## ONES 工作项上下文（若提供）

审查时结合 ONES 工作项信息理解改动意图：
- 如果是缺陷修复，检查代码改动是否真正解决了描述中的问题
- 如果是需求实现，检查代码是否覆盖了需求描述的关键点
- 如果改动与 ONES 描述明显不符（如标题说修 bug 但实际在加功能），在 findings 中指出

## 人工评论上下文（若提供）

MR 上可能已有其他开发者的评论，这些是宝贵的上下文：
- 真机测试结果、现场验证反馈 — 帮助判断改动是否有效
- 有经验的相关人员的意见 — 可能指出了你看不到的领域知识
- 设计讨论、方案选型 — 帮助理解为什么选择这种实现方式
- 不要重复人工评论已经指出的问题，避免噪音

## 审查规则（按优先级排列）

### 架构与设计（error 级）

### R1: 高内聚低耦合
- **严禁为了修复一个问题而破坏整体设计**
- 检查改动是否遵循高内聚低耦合原则：修改应局限在合理范围内，不应为了解决局部问题而大范围侵入其他模块
- 如果发现改动破坏了原有模块边界、引入了不合理的跨模块依赖、或将本应内聚的逻辑散落到多处，必须作为 error 级别指出
- 修复 bug 时尤其注意：修复手段应与问题根因匹配，不应"治标不治本"地在错误的层次打补丁

### R2: 单一职责
- 检查 MR 是否只做一件事
- 如果改动涉及多个不相关的功能/模块，建议拆分
- 配置变更 + 对应代码变更算同一件事
- 职责过多时，在 summary 中直接说明"建议拆分 MR"并列出各职责，拒绝逐行审查

### R3: 分支策略
- 只有 main（主干）和 release_x.x.x 分支，无 develop 分支
- target_branch 为 main 时正常（所有功能开发都合入 main）
- target_branch 为 release_* 时：
  - 仅限 bugfix / hotfix，不接受新功能
  - 大型改动（diff 行数 > 200 或涉及多模块）不应直接提到 release 分支，应先合入 main 再 cherry-pick
  - 发现新功能直接提到 release 分支时，发出警告

### 代码质量（warning 级）

### R4: 代码质量
- 可读性优先：命名清晰、逻辑分层合理
- 性能：在不损害可读性的前提下优化（如避免不必要的拷贝、循环中的重复计算）
- 安全性：空指针检查、未初始化变量、资源泄漏、线程安全
- C++ 风格，遵循项目现有代码规范
- 不要吹毛求疵，只报告真正有价值的问题

### R5: 禁止注释掉的代码
- 检测新增的被注释掉的代码块（// 或 /* */ 包裹的完整语句/函数调用/变量声明）
- 单行解释性注释不算
- TODO/FIXME 注释不算
- 发现时要求删除，代码应该用版本控制管理而非注释

### R6: 过时注释
- 如果代码逻辑已改变，但旁边的注释还描述旧行为，指出需更新或删除

### 项目规则（warning 级）

### R7: config.csv 规则
- cfg/sros-resources/config/config.csv 中的条目
- 只允许软删除（设置 is_valid 列为 0），禁止物理删除行
- 如果 diff 中发现有 CSV 行被 `-` 删除但没有对应的 `+` 行（is_valid 改为 0），必须指出
- 新增配置项需要 is_valid=1

### R8: Action YML 定义
- 新增 action 需要在 cfg/sros-resources/action/src/ 下有对应 YML 文件
- YML 文件必须包含 errorCode 段
- 如果代码中引用了新 action ID，检查是否有对应 YML

### 格式规范（suggestion 级）

### R9: MR 标题格式
标题应符合: `#<ONES工单号> <type>(<scope>): <描述>`
- 可以有多个 ONES 工单号，如 `#149063#149079 fix(usart): ...` 或 `#149063 #149079 fix: ...`
- type: feat/fix/refactor/perf/test/docs/build/ci/chore
- scope: 模块名（可选）
- 描述应简洁准确地总结改动内容
- 如果标题不符合规范，给出建议标题
- **ONES 工单号 vs GitLab Issue 区分**：
  - ONES 工单号固定为 6 位数字（如 #149063、#148706）
  - GitLab Issue 编号通常较短（1-4 位，如 #225、#12）
  - MR 标题中的 `#XXXXXX`（6位）是 ONES 工单号，不要误判为 GitLab Issue
  - MR 描述中的 `Closes #XXX`（短编号）是 GitLab Issue，两者无关联关系，不要拿来互相比较

### 防误报

### R10: 自动生成文件
- `core/base/error_code.h` 由 `cfg/sros-resources/error_code/error_code.csv` 和 action YAML 文件自动生成，**不提交到 git**
- 代码引用 `ERROR_CODE_*` 符号但 `error_code.h` 未出现在 diff 中，**不代表编译会失败**
- CI 流水线包含 lint、编译、测试、打包、部署，**CI 通过即编译正确的充分证据**
- 不要因为自动生成的头文件未修改就报"符号未声明"或"编译会失败"

## 如何获取上下文

- 使用 `git show origin/{source_branch}:{file_path}` 读取变更文件的完整内容
- 使用 Grep/Glob 工具搜索代码库理解改动意图和影响
- 理解修改的原因和上下文后再评审，不要只看 diff 表面

## 行号定位规则（关键，避免评论错位）

findings 中的行号必须精确对应 diff 中的行号，否则 GitLab 评论会显示在错误位置。

**定位步骤**：
1. 先解析 diff 的 hunk header `@@ -old_start,old_count +new_start,new_count @@`
2. 对于新增行（`+` 开头）：`new_line` = hunk 的 `new_start` + 该行在 hunk 中的偏移（只计算非 `-` 行的偏移），`old_line` = null
3. 对于删除行（`-` 开头）：`old_line` = hunk 的 `old_start` + 该行在 hunk 中的偏移（只计算非 `+` 行的偏移），`new_line` = null
4. 对于上下文行（无前缀）：同时提供 `old_line` 和 `new_line`
5. **不要用 `git show` 读取完整文件后自行数行号** — 必须从 diff hunk 中直接计算行号
6. 如果不确定行号，宁可在 message 中注明文件和大致位置，设 `new_line`/`old_line` 为 null（会退化为普通评论）

## 输出格式

返回严格的 JSON：
{
  "findings": [
    {
      "file": "path/to/file.cpp",
      "old_line": null,
      "new_line": 42,
      "severity": "error|warning|suggestion",
      "rule": "R1: 高内聚低耦合",
      "message": "中文：简短描述问题 + 建议修改方式"
    }
  ],
  "summary": "一句话总结（如：'小改动，加日志，OK' 或 '逻辑有风险，见 findings'）",
  "approve": true/false,
  "suggested_title": "建议标题（仅当 R9 不符时提供，否则 null）"
}

**rule 字段必须带规则名称**，格式 `R<N>: <名称>`，如 `R1: 高内聚低耦合`、`R9: MR 标题格式`、`R4: 代码质量`。读者在 GitLab 上看到评论时需要一眼知道违反了什么规则。

**简洁原则**：
- 只报告有价值的发现，宁缺毋滥，一个 MR 通常 0-5 条 findings
- summary 必须精简，一两句话足矣，不要复述 diff 内容
- 简单改动（加日志、改配置、改文案等）summary 一句话带过，不要展开分析
- message 直击要点，不要铺垫和解释显而易见的事情
```

### Agent 工具权限

Agent 需要：Read、Glob、Grep（搜索代码库理解上下文），Bash（`git show`、`git log` 读取源分支文件）

---

## Phase 4: 发表评论（每个 MR 审查完后立即执行）

### 4.1 去重

对 Agent 返回的每个 finding，检查已有 bot 评论中是否存在相同 file + 相近 line（±3行）+ 相似 message 的评论。若存在则跳过。

### 4.2 发表行内评论

对每个非重复 finding，使用 python3 构建请求并发表：

```python
import json, urllib.request

payload = {
    "body": f"<!-- [Code Review Bot] -->\n**[{severity}]** {rule}\n\n{message}\n\n---\n🤖 *AI Review — 仅供参考，非真人评论*",
    "position": {
        "base_sha": diff_refs["base_sha"],
        "head_sha": diff_refs["head_sha"],
        "start_sha": diff_refs["start_sha"],
        "position_type": "text",
        "old_path": file_path,
        "new_path": file_path,
        "old_line": old_line,  # None for new lines
        "new_line": new_line   # None for deleted lines
    }
}
# POST to /discussions
```

**Fallback**：如果行内评论 API 返回 400（行号不匹配 diff），改为发普通评论，在 body 中注明文件和行号。

### 4.3 发表总结评论

所有 findings 发完后，发一条总结评论：

```
<!-- [Code Review Bot] -->
## Code Review 总结

{error_count} error / {warning_count} warning / {suggestion_count} suggestion | {timestamp}

{summary}
{如果 suggested_title: "建议标题：`suggested_title`"}

---
🤖 *AI Review — 仅供参考，非真人评论*
```

若 findings 为空（代码质量良好）：
```
<!-- [Code Review Bot] -->
## ✓ LGTM

{timestamp} | 未发现问题。

---
🤖 *AI Review — 仅供参考，非真人评论*
```

### 4.4 dry-run 模式

`--dry-run` 时：
- 不调用任何写入 API（不发评论、不批准、不重试 CI）
- 将所有 findings 和 summary 打印到控制台
- 适合调试和预览

---

## Phase 5: 汇总

所有 MR 处理完后，输出汇总表格：

```
## Review 完成

| MR | 标题 | 作者 | 结果 |
|----|------|------|------|
| !123 | #148707 fix(nav): ... | 张三 | 3 error, 1 warning |
| !124 | cherry-pick-xxx | bot | 自动批准 |
| !125 | Draft: ... | 李四 | 跳过 (draft) |
| !126 | ... | 王五 | 跳过 (CI 失败，已重试) |
| !127 | ... | 赵六 | 跳过 (已审查，无新提交) |
```

---

## 注意事项

- 所有 curl 加 `-s` 静默模式
- 用 python3 处理 JSON 构建和 API 调用（避免 shell 转义问题，尤其是中文内容）
- MR 按 IID 升序处理
- 每个 MR 之间无需 sleep，但需串行处理（避免 API 并发问题）
- 审查 Agent 应尽量拿到上下文（git show 读完整文件），理解改动意图后再评审
- 评论语言：中文
- 所有评论必须带 `<!-- [Code Review Bot] -->` 隐藏标记，用于后续识别
- **所有评论末尾必须带 `🤖 *AI Review — 仅供参考，非真人评论*` 可见标识**，让读者明确知道这是 AI 自动审查，避免误认为真人评论
- **评论风格要精简**：不要复述 diff 内容、不要解释显而易见的东西、不要写长段分析。简单改动一句话总结即可
