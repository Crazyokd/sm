---
name: tp-run-case
description: Run an existing TP Case and read the verdict from run.error and failed-leaf attempt.error. Use when verifying a case, creating a run, waiting for a run, or asking whether a case passed.
---

# 跑已有 TP Case

只验收，不改编排，不热更新。Server 必须已经在跑。MCP 调已启动的 Server；默认 `TP_BASE_URL=http://127.0.0.1:3000`。

## 何时用

用户要跑某个已有 Case、看过不过、或「再跑一遍」。

## 闭环

**1. 选 Case。** `listCases`。按用户给的名字或 id 对上一项。没有指定就列出让用户选。不要为此新建 Case。

**2. 开跑。** `createBatch` `{ caseIds: [id], trigger: { kind: 'mcp' } }`；多选用非空且唯一的 `caseIds`，跑 Group 时先用 `listGroups` / `getGroup` 确认，再传 `{ groupId, trigger: { kind: 'mcp' } }`。`caseIds` 与 `groupId` 只能选一个。返回一个 Batch，从 `batch.runs` 取得成员 Run 的 id；对每个成员调用 `waitForRun` `{ runId, timeoutMs }`。`timeoutMs` 按 Case 长度给够（含建图/任务的 Case 用三十分钟以上），不要用几秒。

创建时必须显式声明 `trigger`；它是调用方声明的来源，不是认证身份。可选填非空 `name` 和字符串 `description`；不填时 Server 自动命名，说明为空字符串。不要提交 `selection`，由 Server 生成 Case 来源或包含 `groupId` / `groupName` 的 Group 快照。响应、持久化记录及导入都必须带齐 `name`、`description`、`trigger`、`selection`；导入时这四项与 `runs` / `files` 同层，保留原快照。

用 `listBatches` 查批次，`getBatch` `{ id: batch.id }` 查固定成员、状态及计数；用户要停整批时调用 `cancelBatch` `{ id: batch.id }`，再逐个等待成员终态。等待完成后读取各成员的第 3 步证据；批次 `failed` / `canceled` 不能算验收通过，`queued` / `running` 仍未结束。

没有 MCP 时：`POST /api/batches` 仍须显式传 `trigger`，按调用入口声明 `{ kind: 'web' }` 或 `{ kind: 'ci' }`；CI 的可选 `pipelineUrl` 只允许 HTTP(S)。选择器与可选名称、说明规则相同，从响应的 `runs` 逐个订阅 `GET /api/runs/:id/events` 直到 `passed` / `failed` / `canceled`。批次查询用 `GET /api/batches` / `GET /api/batches/:id`，整批停止用 `POST /api/batches/:id/cancel`。没有成员 `GET /api/runs/:id`。

**3. 读结果。**

- `passed`：验收通过。不要再改 Case 或 Adapter。
- `canceled`：等待成功，但验收未完成。
- `failed`：只看 Run 顶栏 `error`，以及 `tree` 里 `type` 为 `Action` / `Condition` 且 `status=failed` 的节点（`capability`、`actor`、最后一条 `attempts[].error`）。

## 禁止

- 没有失败叶子证据就换 Case、改树、或扩 Adapter。
- 为跑 Case 去 `docker run` / `docker rm`，或调用 `deploy-dev.py`。
- 登记 `getRunArtifact`。叶子 `error` 不够时，只有用户要看日志才打开 `data/runs/<runId>/`。
- 活着的 Run（`queued` / `running` / `canceling`）期间 `updateCase` / `deleteCase` 同一条 Case。
