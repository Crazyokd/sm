---
description: '深入分析ONES工作项：双Agent独立分析+分歧裁决，生成根因报告并可选上传'
allowed-tools: Bash(curl, source, cat, mkdir, tar, zstd, gunzip, unzip, find, ls, head, tail, wc, python3, echo, file, du, sed, awk, grep, md5sum, zcat), Read(**), Glob(**), Grep(**), Agent
argument-hint: <工作项ID> [--skip-download] [--no-upload] [--auto]
---

# ONES 工作项深入分析

获取详情 → 下载日志 → 关键词提取 → **双 Agent 独立分析** → 合并比对 → 分歧裁决 → 生成报告 → 可选上传。

## 参数

$ARGUMENTS

- `--skip-download`：跳过附件下载（使用已下载的文件）
- `--no-upload`：不上传报告
- `--auto`：自动模式（被 ones-agent 调用时），跳过确认直接上传

## 前置依赖

认证、API 常量、附件处理策略见 `common.md`。
工作目录：`/tmp/ones:analyze/<工作项ID>/`

---

## Phase 1：获取详情与附件

### Step 1.1：认证
按 `common.md` 执行。

### Step 1.2：并行获取数据
与 `/ones` 相同的 7 路并行请求，获取工作项详情、成员、类型、状态等。创建工作目录：
```bash
mkdir -p /tmp/ones:analyze/<ID>/{images,sros-logs,allspark-logs,other-logs}
```

### Step 1.3：输出问题摘要
格式化表格展示工作项基本信息。

## Phase 1.5：无日志早退检查

检查附件列表中是否有日志文件（`.log`/`.tar`/`.zst`/`.zip`/`.gz`）。

**无日志**（仅图片或无附件）→ 写评论说明 → 结束：
```
[AI Agent] 处理完成
处理类型：现场问题分析
处理结果：该工作项没有日志附件，无法进行深入分析
建议：请补充 SROS 日志后重新触发分析。
```

`--skip-download` 模式跳过此检查。

## Phase 2：附件下载与预处理

按 `common.md` 附件处理策略下载和解压。使用**批量附件 API** 获取签名 URL。

大文件（>50MB 非日志）仅列出不下载。完成后展示文件树。

## Phase 3：关键词提取

从标题、描述、评论、图片中提取关键词，分为：
- **业务关键词**：设备型号、场景、错误现象
- **技术关键词**：变量名、错误码、模块名、日志标签
- **时间范围**：问题发生的具体时间段

## Phase 4：并行双 Agent 独立分析

**同一 turn 内** spawn Agent α 和 Agent β，给予相同输入，各自独立进行代码搜索和日志分析。

每个 Agent 的任务：
1. **代码搜索**：Grep/Read/Glob 搜索错误码、数据流、模块逻辑、配置项
2. **日志分析**：grep 定位后 Read 取上下文（绝不全文读取）
3. **图片分析**：Read 工具查看截图线索
4. **根因推断**：日志行为 + 代码逻辑 → 因果链

输出 JSON 结构化结果：`root_cause`, `category`, `confidence`, `causal_chain`, `evidence`, `key_code_paths`, `fix_suggestions`, `timeline`

**关键原则**：
- 不将工作项中已有的"原因分析"作为直接证据
- 每个结论标注 `[直接证据]` 或 `[间接推理]`

## Phase 5：合并与分歧识别

比对两个 Agent 结果：
- **一致点** → 直接采纳，标记"双重验证"
- **互补点** → 合并补充
- **分歧点** → 进入 Phase 6

## Phase 6：分歧裁决（条件执行）

仅当有分歧时 spawn Agent γ：
- 独立验证双方引用的证据
- 评估证据权重（直接 > 间接，日志 > 纯代码推理）
- 给出裁决和置信度

## Phase 7：生成报告与上传

### 报告格式（HTML）

必须包含：
1. AI 声明横幅（见 `common.md`）
2. 证据标记（直接证据/间接推理）
3. 标准结构：根因 → 因果链 → 时间线 → 证据 → 分析过程 → 影响范围 → 修复建议 → 关键代码路径

输出到 `/tmp/ones:analyze/<ID>/report.html`

### 上传

- `--auto`：直接上传
- 默认：展示报告，等用户确认
- `--no-upload`：仅生成

上传方式：读取现有 `desc_rich` → 拼接 `<hr/>` + 报告 → 写回（见 `common.md` 更新 API）。
