---
name: tp-run-case
description: Run an existing TP Case and read the verdict from run.error and failed-leaf attempt.error. Use when verifying a case, creating a run, waiting for a run, or asking whether a case passed.
---

# 跑已有 TP Case

只验收，不改编排，不热更新。Server 必须已经在跑。MCP 调已启动的 Server；默认 `TP_BASE_URL=http://127.0.0.1:3000`。

## 何时用

用户要跑某个已有 Case、看过不过、或「再跑一遍」。改 EasyGo 源码后再验收，先走 `easygo-tp-hotupdate-verify`，再回到本 skill 的第 2 步。

## 闭环

**1. 选 Case。** `listCases`。按用户给的名字或 id 对上一项。没有指定就列出让用户选。不要为此新建 Case。

**2. 开跑。** `createRun` `{ caseId }`。再 `waitForRun` `{ runId, timeoutMs }`。`timeoutMs` 按 Case 长度给够（含建图/任务的 EasyGo Case 用三十分钟以上），不要用几秒。没有 MCP 时：`POST /api/runs`，再订 `GET /api/runs/:id/events` 直到 `passed` / `failed` / `canceled`。没有成员 `GET /api/runs/:id`。

**3. 读结果。**

- `passed`：验收通过。不要再改 Case 或 Adapter。
- `canceled`：等待成功，但验收未完成。
- `failed`：只看 Run 顶栏 `error`，以及 `tree` 里 `type` 为 `Action` / `Condition` 且 `status=failed` 的节点（`capability`、`actor`、最后一条 `attempts[].error`）。

## 禁止

- 没有失败叶子证据就换 Case、改树、或扩 Adapter。
- 为跑 Case 去 `docker run` / `docker rm`，或调用 `deploy-dev.py`。
- 登记 `getRunArtifact`。叶子 `error` 不够时，只有用户要看日志才打开 `data/runs/<runId>/`。
- 活着的 Run（`queued` / `running` / `canceling`）期间 `updateCase` / `deleteCase` 同一条 Case。
