---
name: feishu-notify
description: 把批次汇总 / 通知卡片推送到飞书群,并自动 @ 涉及到的人。适用于 autopilot 批次完成后播报、CI 报告、值班同步等场景,也覆盖 feishu、lark、飞书、群通知、卡片、@人、batch_get_id、chats/search、tenant_access_token 等触发词。
---

# feishu-notify

把结构化汇总 (`summary.json`) 推送成飞书 interactive 富文本卡片到指定群,并按"邮箱 → 群成员中文名"自动解析 @ 人员。**按 `assignee_account` 分桶,每个问题解决人各发一张只含他自己缺陷的卡**(调用方即使把整批 items 一起传进来也会被正确拆分;单桶失败不阻塞其它桶)。

## 何时用

- autopilot 批次跑完后,需要把摘要+ @ 处理人推送到飞书群
- 任何"把结果同时回写到外部系统 + 通知群"的场景中,做飞书侧那一脚
- 不要用本 skill 发普通文本聊天消息;它专注 interactive 卡片 + 自动 @ 解析
- 底层 SDK 已抽出为 `scripts/feishu_lark.py`(鉴权 / chat_id 解析 / batch_get_id / resolve_mentions / send 等),CI 或其他自动化脚本(如 `scripts/ci_auto_backport.py`)可 `from feishu_lark import ...` 复用,自写卡片 schema 即可

## 一键调用

```bash
# summary.json 走 stdin 或文件
python3 .agents/skills/feishu-notify/scripts/feishu_notify.py summary.json
cat summary.json | python3 .agents/skills/feishu-notify/scripts/feishu_notify.py -
```

成功 → stdout 每行一个 `message_id`(按问题解决人分桶各发一张),至少一桶成功即退出码 `0`。
凭证缺失 → 退出码 `10`,stderr 给原因 (不算失败,调用方应自己决定是否报错)。
发送失败 → 退出码 `20`,stderr 给原因。

## 凭证 (按优先级,先非空者赢)

1. 进程环境变量
2. `$FEISHU_ENV_FILE` 指向的 `KEY=VALUE` 文件
3. `~/.config/sros-autopilot/feishu.env` (推荐,用户家目录,权限 600,不进 git)
4. `<skill 同目录>/.feishu.env.local` (项目内本地,已加入 .gitignore)

必填:

- `FEISHU_APP_ID`
- `FEISHU_APP_SECRET`
- `FEISHU_AUTOPILOT_CHAT_ID` (`oc_xxx`) 或 `FEISHU_AUTOPILOT_CHAT_NAME` (群名,二选一)
    - CHAT_ID 优先生效,无歧义最快
    - 只配 CHAT_NAME 时启动期调 `/im/v1/chats/search` 精确匹配换出 chat_id;重名会报错让你二选一;机器人必须已在群里

## 飞书应用权限 (缺一不可)

- `im:message:send_as_bot` — 发卡片到群
- `contact:user.email:readonly` — 邮箱反查 open_id (自动 @ 第 2 步)
- `im:chat:readonly` — 列出群成员 (自动 @ 第 3 步 + chat_name → chat_id 换算)

应用要被加入到目标群,否则 `chats/search` 返回空候选 / `send` 报 `230002 Bot is not in the chat`。

## summary.json schema

```json
{
  "trigger_issue_id": "150962",
  "stats": {
    "total": 5, "formal_mr": 3, "draft_mr": 1,
    "main_fixed": 0, "cherry_picked": 0, "pending_cherry_pick": 0,
    "need_clarify": 1, "need_planning": 0, "failed": 0
  },
  "items": [
    {
      "ones_id": "150962",
      "title": "缺陷标题 (卡片明细 >35 字会截断)",
      "result": "formal_mr|draft_mr|main_fixed|cherry_picked|pending_cherry_pick|need_clarify|need_planning|failed",
      "mr_url": "https://gitlab.../merge_requests/123",
      "ones_url": "https://ones.standard-robots.com:10120/project/#/team/UNrQ5Ny5/task/<task_uuid>",
      "assignee_account": "Skyler",
      "assignee_name": "万里翔",
      "assignee_email": "wlx@example.com"
    }
  ]
}
```

调用方在生成 summary 时,**强烈建议**对每条缺陷顺手填 `assignee_email`(从 ONES 取),@ 解析命中率从 ~60% 提升到 ~100%。

**`ones_url` 必须用 task uuid 拼**:格式 `https://ones.standard-robots.com:10120/project/#/team/UNrQ5Ny5/task/<task_uuid>`。`<task_uuid>` 是 ONES 接口返回的 `uuid` 字段(形如 `Uea8vXdNkwqq2m0Q`),**不是** `#151157` 这种数字编号——用数字编号拼出的链接打开会失效。`/project/#/` 里的 `#` 不能漏。

## @ 自动解析顺序

底层 `resolve_mentions()`（feishu_lark.py）只跑前 3 步，返回 `(resolved, missing)`，
不内置任何兜底；调用方各自决定第 4 步行为。

1. **人工 override** — 同目录 `feishu_user_map.json`(可选,默认无此文件)。
2. **邮箱 `batch_get_id`** — `POST /open-apis/contact/v3/users/batch_get_id?user_id_type=open_id`,主路径,推荐填好邮箱用这步。
3. **群成员中文名匹配** — `GET /open-apis/im/v1/chats/<chat_id>/members`,按 `assignee_name` 精确匹配。
4. **调用方兜底**（行为按 caller 决定）：
    - `feishu_notify.py` CLI（autopilot 批次卡）→ `<at id="all"></at>` 兜底 + note 区列出未映射的中文名
    - `scripts/ci_auto_backport.py`（backport 失败通知）→ **不**走 `@all`（大群噪声），改为在卡片显示「相关人 @ 解析失败（未映射：…）」+ 引导维护 `feishu_user_map.json`

## 卡片视觉规则

- header template: `red` (失败>0 或 产出率<1.0) > `yellow` (需澄清 / 需立项 / 待 cherry-pick `pending_cherry_pick` 任一 >0) > `green` (其余，即全部产出或已解决)
- 产出率 = (formal_mr + draft_mr) / (total − 已解决项 − 待办项)，分母剔除 `main_fixed` / `cherry_picked`（已解决）与 `pending_cherry_pick` / `need_clarify` / `need_planning`（待办），避免它们被误算成"没产出 MR"拉低产出率；但待办项仍触发 `yellow`
- 明细 div 超过 20 条 → 前 5 条 + 折叠提示
- actions (button) 最多 5 个,优先级: 失败 > 需澄清 > 需立项 > MR 链接
- 卡片体积超 30KB 会被飞书拒,脚本会自动只留统计行 + actions

## 错误码自查

- `230001 invalid receive_id` — chat_id 写错,或 CHAT_NAME 反查没命中
- `230002 Bot is not in the chat` — 机器人没加进群
- `99991663 / 99991664` — token 失效或权限缺失,检查应用权限范围
- `99991668 user not found` — `batch_get_id` 邮箱在飞书查不到,会自动降级到群成员名匹配,无需处理

## 不要做的事

- 不要修改脚本去发普通文本聊天 / 群消息;本 skill 只发 interactive 卡片
- 不要把凭证写到脚本或 prompt 里;一律走 §"凭证"那 4 个来源
- 不要在 summary 里手工 @ open_id;让脚本通过邮箱/中文名自动解析
- 不要重试发送 (脚本内部已重试 token;某个桶发送失败即跳过该桶、继续其它桶,不重试,调用方自己决定是否再整跑)
