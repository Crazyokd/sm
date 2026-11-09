---
name: tp-edit-case
description: Create or update a TP Case from the adapter catalog. Use when editing a behavior tree, adding a leaf, changing actor or session ports, or saving a case definition.
---

# 按目录改 TP Case

编排只使用已注册 Adapter 的 capability 和 Session 口。非法输入直接失败，不要补默认值或兼容旧字段。

## 何时用

用户要新建/改 Case、加叶子、改 actor、改 Session 口。只要跑已有 Case，走 `tp-run-case`。

## 闭环

**1. 读目录。** `listAdapters`。要对某一 Adapter 的口，再 `getAdapter`。叶子的 `adapter` / `capability` / 口名必须来自这份目录。Session 口名必须来自该 Adapter 的 `session.ports`。不要发明 capability 或口。

**2. 读或起 Case。** 已有则 `getCase`。新建则先在本地拼 `definition`，最后 `createCase`。不要用 `createCaseAndRun` 除非用户明确要创建后立刻跑。

**3. 编树。**

- 叶子手填 `actor`。同一 `(adapter, actor)` 共用一份 Session。
- 口只接线：`{ literal }` 或 `{ key }`。不要写未声明的口。
- `definition.sessions` 必须精确覆盖树上每个 `(adapter, actor)`。多一行、少一行、口不合 schema 都会 400。
- 改叶子上的 adapter/actor 后重算 sessions：已有对的口保留，新对出口为空对象，树上没了的行丢掉。
- Sim 必填 session 口 `world` / `appImage` / `robots`（字面量）。Browser 也要有一行，口可以为空。

**4. 保存。** `updateCase` 或 `createCase`。400 / `invalid`：对照 `message` 改 definition，再保存。不要降级、不要删 `sessions`、不要换没在目录里的 capability。

有该 Case 的活 Run 时保存会失败。先 `cancelRun` 等到终态，或等跑完，再保存。

## 禁止

- 发明目录里没有的 capability、口、或 Adapter。
- 给 Session 口加版本 / SHA / 镜像校验，或让 Session 口读黑板（第一期只允许字面量）。
- 把具体 Adapter / 产品概念写进 `packages/server/` / `packages/contract/`。
- 把 Case 源码写进 Adapter。仓库里的树在 `packages/cases/`。
- 保存失败后改 MCP、改契约、或「先跑起来再说」。
