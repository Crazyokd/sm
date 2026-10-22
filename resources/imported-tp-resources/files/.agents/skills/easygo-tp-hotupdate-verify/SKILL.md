---
name: easygo-tp-hotupdate-verify
description: After changing EasyGo source, deploy-dev into a live container then verify with an existing TP Case (createRun / waitForRun / failed-leaf errors). Use when confirming a fix, re-running acceptance, or iterating 热更新 → 再跑.
---

# EasyGo 热更新后用 TP Case 验收

跨两个仓的两步环。热更新在 `~/workspace/easygo`，验收在本仓库的 TP。平台不调用 deploy，不校验 EasyGo 版本。

## 何时用

改了 EasyGo 源码，要用**已有** TP Case 判断修没修好；或用户说「再跑验收 / 确认修复 / 热更新后再测」。

## 先决条件

- EasyGo 工作区：`~/workspace/easygo`（以用户给出的路径为准）。
- TP Server 已在跑。MCP 调已启动的 Server；默认 `TP_SERVER_URL=http://127.0.0.1:3000`。
- Case 已存在。从 Case 读 `definition.sessions` 里 EasyGo 行的 `ports.container`，**不要猜容器名**。
- 目标容器已在跑。`deploy-dev.py` 热装进已有容器；TP Session 只 `docker inspect` 附着，不 `docker run` / `rm`。
- 未授权不往真机或陌生容器热装。

## 闭环

**1. 读 Case。** `getCase`（MCP）或 `GET /api/cases/:id`。记下 `caseId` 和 EasyGo session 的 `container`。该字面量必须和接下来 `deploy-dev.py -c` 一致。

**2. 热更新。** 在 EasyGo 仓跑 `deploy-dev.py`，等它结束（停 `ros2-launch`、热装、再启）：

```bash
# 本机仿真容器（架构必须 amd64）
python3 script/deploy-dev.py localhost <repo> [repo...] --arch amd64 -c <container>

# 真机（默认 arch=arm64，容器默认 easygo，务必改成 Case 里的名字）
python3 script/deploy-dev.py <SSH-host> <repo> [repo...] -c <container>
```

`<repo>` 是仓库名（`easygo`、`easygo-app`、`vda5050`、`slam`、`navigation2`…），不是 colcon 包名。只改了哪个仓就热哪个。`--list` 可列仓库。未授权真机则停在本机仿真。

**3. 开跑。** `createRun` `{ caseId }`，再 `waitForRun` `{ runId, timeoutMs }`。`timeoutMs` 按 Case 长度给够（Full Journey 量级用三十分钟以上），不要用几秒超时。没有 MCP 时：`POST /api/runs`，再订 `GET /api/runs/:id/events` 直到 `passed` / `failed` / `canceled`。没有成员 `GET /api/runs/:id`。

`createSession` 会等到 EasyGo API ready。必须等 `deploy-dev.py` 退出后再 `createRun`。

**4. 读失败。** 只看这些，据此改下一刀 EasyGo 代码：

- Run 顶栏 `error`（来自最晚失败叶子的 `attempt.error`）
- `tree` 里 `type` 为 `Action` / `Condition` 且 `status=failed` 的节点：`capability`、`actor`、最后一条 `attempts[].error`

**5. 再改再跑。** 改 EasyGo 后回到第 2 步。不改编排，除非用户要求改 Case。

## 禁止

- 让 TP 调用 `deploy-dev.py`，或在 Session 口加版本 / SHA / 镜像校验。
- 为跑 Case 去 `docker run` / `docker rm`，或派生一份新仿真。
- 没有失败叶子证据就换 Case、改树、或扩 Adapter。
- 把脏仿真（残留地图 / 任务 / 定位）当成平台缺陷。收尾写在 Case 树上（`map.delete`、`task.delete`、`localization.cancel`、`reset_spawn`），由用户决定要不要编。
- 把 EasyGo 概念写进 `packages/server/` / `packages/contract/`。

## 证据不够时

叶子 `error` 和 Run `error` 是验收依据。只有用户要看日志时，才打开该 Run 的 `data/runs/<runId>/`（`steps.log`、`container.log`、`bridge.log`）。不要为了「更完整」去改 MCP 或加版本口。
