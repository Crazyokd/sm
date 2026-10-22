#!/usr/bin/env python3
"""autopilot §7 飞书播报最小实现。

按 `assignee_account` 把 summary.items 分桶，**每个问题解决人各发一张只含他自己缺陷的卡**
（调用方即使把整批 items 一起传进来也会被正确拆分；单桶发送失败不阻塞其它桶）。

用法:
    python3 feishu_notify.py <summary.json>
    cat summary.json | python3 feishu_notify.py -

依赖: 仅标准库 (urllib)。

环境变量:
    FEISHU_APP_ID
    FEISHU_APP_SECRET
    FEISHU_AUTOPILOT_CHAT_ID    或    FEISHU_AUTOPILOT_CHAT_NAME (二选一)
        CHAT_ID 优先 (oc_xxx, 一次定位无歧义);
        只配 CHAT_NAME 时启动期调 /im/v1/chats/search 精确匹配群名换 chat_id,
        重名会报错让你二选一。

环境变量来源 (按优先级, 先非空者赢):
    1. 进程环境
    2. $FEISHU_ENV_FILE 指向的 KEY=VALUE 文件
    3. ~/.config/sros-autopilot/feishu.env   (推荐, 用户家目录, 不进 git)
    4. <脚本同目录>/.feishu.env.local        (项目内本地, 已加入 .gitignore)
凭证文件建议权限 600 (chmod 600), 文件内任何 # 开头的行被忽略。

人员映射: 与本脚本同目录的 feishu_user_map.json (可选)。

summary.json schema:
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
      "title": "...",                       // 卡片明细最多展示 35 字
      "result": "formal_mr|draft_mr|main_fixed|cherry_picked|pending_cherry_pick|need_clarify|need_planning|failed",
      "mr_url": "https://...",
      "ones_url": "https://ones.standard-robots.com:10120/project/#/team/UNrQ5Ny5/task/<task_uuid>",  // 必须用 task uuid 不是数字编号
      "assignee_account": "Skyler",          // ONES account_name, 用作 override key
      "assignee_name": "万里翔",               // ONES display name, 用于群成员匹配
      "assignee_email": "wlx@example.com"    // ONES email, 用于 batch_get_id (强烈建议提供)
    }
  ]
}

@ 解析顺序: feishu_user_map.json override → 邮箱 batch_get_id → 群成员中文名匹配 → @all 兜底
需要的飞书权限: im:message:send_as_bot, contact:user.email:readonly, im:chat:readonly

退出码:
    0  成功 (stdout 每行一个 message_id, 按问题解决人分桶各发一张; 至少一桶成功即 0)
    10 跳过 (凭证缺失, stderr 给原因)
    20 失败 (stderr 给原因)
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
from pathlib import Path

# 底层 SDK 抽到 feishu_lark，便于其他自动化（如 ci_auto_backport.py）import 复用。
# batch_get_id_by_email / list_group_members 由 resolve_mentions 内部调用，本文件不再直接用。
from feishu_lark import (
    get_token,
    load_env_file,
    load_user_map,
    resolve_chat_id_by_name,
    resolve_mentions,
    send,
)

RESULT_EMOJI = {
    'formal_mr': '✅',
    'draft_mr': '📝',
    'main_fixed': '⏭',
    'cherry_picked': '🍒',
    'pending_cherry_pick': '🔁',
    'need_clarify': '❓',
    'need_planning': '🏗',
    'failed': '❌',
}


def pick_template(stats: dict) -> tuple[str, float]:
    n = stats.get('total', 0)
    fixed = stats.get('main_fixed', 0)
    cp = stats.get('cherry_picked', 0)
    pcp = stats.get('pending_cherry_pick', 0)
    q = stats.get('need_clarify', 0)
    L = stats.get('need_planning', 0)
    failed = stats.get('failed', 0)
    # 已解决项（main_fixed / cherry_picked）与待办项（pending_cherry_pick / 需澄清 /
    # 需立项）都不计入产出率分母，避免把它们误算成"没产出 MR"而拉低产出率。
    denom = max(n - fixed - cp - pcp - q - L, 0)
    produced = stats.get('formal_mr', 0) + stats.get('draft_mr', 0)
    rate = (produced / denom) if denom else 1.0
    if failed > 0 or rate < 1.0:
        return 'red', rate
    # 待 cherry-pick 不拖累产出率，但仍是待办（main 已合、release 未补成），不判绿
    if q > 0 or L > 0 or pcp > 0:
        return 'yellow', rate
    return 'green', rate


def _truncate(s: str, n: int) -> str:
    s = s or ''
    return s if len(s) <= n else s[: n - 1] + '…'


def collect_assignees(items: list[dict]) -> list[dict]:
    """从 items 抽取去重的 assignee 三元组 (account, name, email)，保持出现顺序。"""
    out: list[dict] = []
    seen: set[str] = set()
    for it in items:
        acct = it.get('assignee_account')
        if not acct or acct in seen:
            continue
        seen.add(acct)
        out.append(
            {
                'account': acct,
                'name': it.get('assignee_name') or acct,
                'email': it.get('assignee_email') or '',
            }
        )
    return out


def build_card(summary: dict, resolved: list[dict], missing: list[str]) -> dict:
    stats = summary.get('stats', {})
    items = summary.get('items', [])
    trigger = summary.get('trigger_issue_id', '-')
    template, rate = pick_template(stats)

    # 分组统计：只展示非 0 类别
    def grp(label: str, pairs: list[tuple[str, int]]) -> str | None:
        shown = [f'{name} {n}' for name, n in pairs if n]
        return f'{label}：' + ' · '.join(shown) if shown else None

    stat_rows = [f'**总数 {stats.get("total", 0)}**  ·  产出率 **{rate:.2f}**']
    for row in (
        grp(
            '🟢 产出', [('正式MR', stats.get('formal_mr', 0)), ('Draft', stats.get('draft_mr', 0))]
        ),
        grp(
            '⏭ 已修复',
            [
                ('main', stats.get('main_fixed', 0)),
                ('已cherry-pick', stats.get('cherry_picked', 0)),
            ],
        ),
        grp(
            '🟡 待办',
            [
                ('需澄清', stats.get('need_clarify', 0)),
                ('需立项', stats.get('need_planning', 0)),
                ('待cherry-pick', stats.get('pending_cherry_pick', 0)),
            ],
        ),
        grp('🔴 失败', [('失败', stats.get('failed', 0))]),
    ):
        if row:
            stat_rows.append(row)
    stat_md = '\n'.join(stat_rows)

    items_sorted = sorted(items, key=lambda x: str(x.get('ones_id', '')))
    lines: list[str] = []
    for it in items_sorted[:20]:
        emoji = RESULT_EMOJI.get(it.get('result', ''), '·')
        ones_id = it.get('ones_id', '-')
        title = _truncate(it.get('title', ''), 35)
        links = []
        if it.get('mr_url'):
            links.append(f'[MR]({it["mr_url"]})')
        if it.get('ones_url'):
            links.append(f'[ONES]({it["ones_url"]})')
        tail = ('  ·  ' + '  ·  '.join(links)) if links else ''
        lines.append(f'{emoji} **#{ones_id}** {title}{tail}')
    extra = max(len(items_sorted) - 20, 0)
    detail_md = '\n'.join(lines) if lines else '（本批次无命中）'
    if extra:
        detail_md += f'\n…（其余 {extra} 条折叠，详情见 ONES 触发 issue 评论）'

    mentioned = [f'<at id="{r["open_id"]}">{r["name"]}</at>' for r in resolved]
    if not mentioned:
        mentioned.append('<at id="all"></at>')
    mention_md = '相关人：' + ' '.join(mentioned)

    priority = {'failed': 0, 'need_clarify': 1, 'need_planning': 2}
    action_items = sorted(
        (i for i in items if i.get('result') in priority and i.get('ones_url')),
        key=lambda i: priority[i['result']],
    )
    mr_items = [
        i for i in items if i.get('result') in ('formal_mr', 'draft_mr') and i.get('mr_url')
    ]

    buttons: list[dict] = []
    for it in action_items[:5]:
        buttons.append(
            {
                'tag': 'button',
                'text': {
                    'tag': 'plain_text',
                    'content': f'{RESULT_EMOJI.get(it["result"], "·")} #{it["ones_id"]}',
                },
                'type': 'default',
                'url': it['ones_url'],
            }
        )
    for it in mr_items:
        if len(buttons) >= 5:
            break
        buttons.append(
            {
                'tag': 'button',
                'text': {'tag': 'plain_text', 'content': f'MR #{it["ones_id"]}'},
                'type': 'primary',
                'url': it['mr_url'],
            }
        )

    notes: list[str] = []
    if missing:
        notes.append(f'未映射 ONES 账号：{", ".join(missing)}')
    notes.append(f'触发 issue：#{trigger}')

    elements: list[dict] = [
        {'tag': 'div', 'text': {'tag': 'lark_md', 'content': stat_md}},
        {'tag': 'hr'},
        {'tag': 'div', 'text': {'tag': 'lark_md', 'content': detail_md}},
        {'tag': 'hr'},
        {'tag': 'div', 'text': {'tag': 'lark_md', 'content': mention_md}},
    ]
    if buttons:
        elements.append({'tag': 'action', 'actions': buttons})
    elements.append(
        {
            'tag': 'note',
            'elements': [{'tag': 'plain_text', 'content': ' · '.join(notes)}],
        }
    )

    # 本卡片对应一个问题解决人桶（main() 已按 assignee_account 把 items 分桶后逐桶调
    # build_card）。副标题显示该桶的 assignee_name（同桶内一致），没有则回退到已解析
    # 的 @ 名单首位。
    assignee = next(
        (it.get('assignee_name') for it in items if it.get('assignee_name')),
        (resolved[0]['name'] if resolved else '未指派'),
    )

    return {
        'config': {'wide_screen_mode': True},
        'header': {
            'template': template,
            'title': {'tag': 'plain_text', 'content': 'SROS 缺陷自动修复 · 批次汇总'},
            'subtitle': {
                'tag': 'plain_text',
                'content': f'{time.strftime("%Y-%m-%d %H:%M")}  ·  问题解决人：{assignee}',
            },
        },
        'elements': elements,
    }


def bucket_items(items: list[dict]) -> dict[str, list[dict]]:
    """按 assignee_account 分桶，保持首次出现顺序。无 account 的归 __unassigned__。

    items 为空时返回 {'__all__': []}，保证至少发一张（空批次）汇总卡。
    """
    if not items:
        return {'__all__': []}
    buckets: dict[str, list[dict]] = {}
    for it in items:
        key = it.get('assignee_account') or '__unassigned__'
        buckets.setdefault(key, []).append(it)
    return buckets


def bucket_stats(items: list[dict]) -> dict:
    """从桶内 items 的 result 字段重算 stats（result 枚举与 stats 键一一对应）。"""
    stats: dict[str, int] = {'total': len(items)}
    for it in items:
        r = it.get('result', '')
        if r:
            stats[r] = stats.get(r, 0) + 1
    return stats


def main() -> int:
    load_env_file()
    app_id = os.environ.get('FEISHU_APP_ID')
    app_secret = os.environ.get('FEISHU_APP_SECRET')
    chat_id = os.environ.get('FEISHU_AUTOPILOT_CHAT_ID')
    chat_name = os.environ.get('FEISHU_AUTOPILOT_CHAT_NAME')
    missing_env: list[str] = []
    if not app_id:
        missing_env.append('FEISHU_APP_ID')
    if not app_secret:
        missing_env.append('FEISHU_APP_SECRET')
    if not chat_id and not chat_name:
        missing_env.append('FEISHU_AUTOPILOT_CHAT_ID 或 FEISHU_AUTOPILOT_CHAT_NAME')
    if missing_env:
        print(f'skip: missing env: {", ".join(missing_env)}', file=sys.stderr)
        return 10

    if len(sys.argv) != 2:
        print('usage: feishu_notify.py <summary.json|->', file=sys.stderr)
        return 20

    src = sys.argv[1]
    try:
        raw = sys.stdin.read() if src == '-' else Path(src).read_text(encoding='utf-8')
        summary = json.loads(raw)
    except (OSError, json.JSONDecodeError) as exc:
        print(f'failed: bad summary: {exc}', file=sys.stderr)
        return 20

    try:
        token = get_token(app_id, app_secret)
        if not chat_id:
            chat_id = resolve_chat_id_by_name(token, chat_name)
            print(f'resolved chat_id={chat_id} for name={chat_name!r}', file=sys.stderr)
    except urllib.error.HTTPError as exc:
        body = exc.read().decode('utf-8', 'replace')[:200]
        print(f'failed: http {exc.code} {body}', file=sys.stderr)
        return 20
    except Exception as exc:
        print(f'failed: {exc}', file=sys.stderr)
        return 20

    # 按问题解决人 (assignee_account) 分桶，逐桶各发一张只含该人缺陷的卡片。
    # 单桶失败不阻塞其它桶（§7.3）；调用方即使把整批 items 一起传进来也会被正确拆分。
    user_map = load_user_map()
    trigger = summary.get('trigger_issue_id', '-')
    buckets = bucket_items(summary.get('items', []))
    sent: list[str] = []
    failed: list[str] = []
    for acct, bitems in buckets.items():
        try:
            sub = {'trigger_issue_id': trigger, 'stats': bucket_stats(bitems), 'items': bitems}
            assignees = collect_assignees(bitems)
            resolved, unresolved = resolve_mentions(token, chat_id, assignees, user_map)
            card = build_card(sub, resolved, unresolved)
            mid = send(token, chat_id, card)
            sent.append(mid)
            print(f'bucket {acct}: message_id={mid} ({len(bitems)} 条)', file=sys.stderr)
        except urllib.error.HTTPError as exc:
            body = exc.read().decode('utf-8', 'replace')[:200]
            failed.append(f'{acct}: http {exc.code} {body}')
            print(f'bucket {acct}: FAILED http {exc.code} {body}', file=sys.stderr)
        except Exception as exc:
            failed.append(f'{acct}: {exc}')
            print(f'bucket {acct}: FAILED {exc}', file=sys.stderr)

    for mid in sent:
        print(mid)
    if failed and not sent:
        return 20
    return 0


if __name__ == '__main__':
    sys.exit(main())
