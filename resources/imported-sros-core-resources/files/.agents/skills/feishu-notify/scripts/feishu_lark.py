#!/usr/bin/env python3
"""飞书底层 SDK 模块（可 import 复用）。

抽离自 feishu_notify.py，供 CI/自动化脚本（如 scripts/ci_auto_backport.py）共享
鉴权、群解析、@人解析、消息发送能力，避免重复造轮子。

仅依赖标准库（urllib）。任何上层卡片业务（build_card / pick_template /
bucket_items 等）保留在 feishu_notify.py，不进本模块。

环境变量来源（按优先级，先非空者赢）：
    1. 进程环境
    2. $FEISHU_ENV_FILE 指向的 KEY=VALUE 文件
    3. ~/.config/sros-autopilot/feishu.env
    4. <脚本同目录>/.feishu.env.local

人员映射（可选）：与本模块同目录的 feishu_user_map.json。
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

FEISHU = 'https://open.feishu.cn'
SCRIPT_DIR = Path(__file__).resolve().parent
USER_MAP_FILE = SCRIPT_DIR / 'feishu_user_map.json'


def _http(
    method: str, url: str, body: dict | None = None, token: str | None = None, timeout: int = 10
) -> dict:
    headers = {'Content-Type': 'application/json; charset=utf-8'}
    if token:
        headers['Authorization'] = f'Bearer {token}'
    data = json.dumps(body, ensure_ascii=False).encode('utf-8') if body is not None else None
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode('utf-8'))


def get_token(app_id: str, app_secret: str) -> str:
    last_exc: Exception | None = None
    for attempt in (1, 2):
        try:
            res = _http(
                'POST',
                f'{FEISHU}/open-apis/auth/v3/tenant_access_token/internal',
                {'app_id': app_id, 'app_secret': app_secret},
            )
            if res.get('code') == 0 and res.get('tenant_access_token'):
                return res['tenant_access_token']
            raise RuntimeError(f'token code={res.get("code")} msg={res.get("msg")}')
        except Exception as exc:
            last_exc = exc
            if attempt == 1:
                time.sleep(2)
    raise RuntimeError(f'get_token failed: {last_exc}')


def load_env_file() -> None:
    """从本地凭证文件加载 KEY=VALUE 到 os.environ（不覆盖已存在的进程环境变量）。"""
    candidates: list[Path] = []
    explicit = os.environ.get('FEISHU_ENV_FILE')
    if explicit:
        candidates.append(Path(explicit).expanduser())
    candidates.append(Path.home() / '.config' / 'sros-autopilot' / 'feishu.env')
    candidates.append(SCRIPT_DIR / '.feishu.env.local')
    for path in candidates:
        if not path.is_file():
            continue
        try:
            for raw_line in path.read_text(encoding='utf-8').splitlines():
                line = raw_line.strip()
                if not line or line.startswith('#'):
                    continue
                if '=' not in line:
                    continue
                key, _, value = line.partition('=')
                key = key.strip()
                value = value.strip().strip('"').strip("'")
                if key and key not in os.environ:
                    os.environ[key] = value
        except OSError as exc:
            print(f'warn: cannot read env file {path}: {exc}', file=sys.stderr)


def load_user_map() -> dict:
    if not USER_MAP_FILE.exists():
        return {}
    try:
        raw = json.loads(USER_MAP_FILE.read_text(encoding='utf-8'))
    except json.JSONDecodeError as exc:
        print(f'warn: bad user map: {exc}', file=sys.stderr)
        return {}
    return {k: v for k, v in raw.items() if not k.startswith('_')}


def resolve_chat_id_by_name(token: str, name: str) -> str:
    """通过群名精确匹配换出 chat_id。"""
    url = f'{FEISHU}/open-apis/im/v1/chats/search?query={urllib.parse.quote(name)}&page_size=20'
    res = _http('GET', url, token=token)
    if res.get('code') != 0:
        raise RuntimeError(f'chats/search code={res.get("code")} msg={res.get("msg")}')
    items = (res.get('data', {}) or {}).get('items', []) or []
    exact = [c for c in items if c.get('name') == name and c.get('chat_id')]
    if not exact:
        cands = [c.get('name') for c in items[:5]]
        raise RuntimeError(
            f'no exact chat with name "{name}"; bot-visible candidates: {cands} '
            f'(check if bot was added to the chat, or set FEISHU_AUTOPILOT_CHAT_ID directly)'
        )
    if len(exact) > 1:
        ids = [c.get('chat_id') for c in exact]
        raise RuntimeError(
            f'multiple chats named "{name}": {ids}; set FEISHU_AUTOPILOT_CHAT_ID to disambiguate'
        )
    return exact[0]['chat_id']


def list_group_members(token: str, chat_id: str) -> dict[str, str]:
    """返回群成员 `name -> open_id` 映射，自动分页。失败返回空 dict（不阻塞主流程）。"""
    members: dict[str, str] = {}
    page_token: str | None = None
    base = f'{FEISHU}/open-apis/im/v1/chats/{chat_id}/members?member_id_type=open_id&page_size=100'
    for _ in range(20):  # 最多 2000 人，防御性上限
        url = base + (f'&page_token={page_token}' if page_token else '')
        try:
            res = _http('GET', url, token=token)
        except Exception as exc:
            print(f'warn: list_group_members failed: {exc}', file=sys.stderr)
            return members
        if res.get('code') != 0:
            print(
                f'warn: list_group_members code={res.get("code")} msg={res.get("msg")}',
                file=sys.stderr,
            )
            return members
        data = res.get('data', {}) or {}
        for m in data.get('items', []) or []:
            name = m.get('name')
            oid = m.get('member_id')
            if name and oid:
                members.setdefault(name, oid)
        page_token = data.get('page_token')
        if not page_token:
            break
    return members


def batch_get_id_by_email(token: str, emails: list[str]) -> dict[str, str]:
    """飞书 batch_get_id，返回 `email -> open_id`。失败返回空 dict。"""
    if not emails:
        return {}
    try:
        res = _http(
            'POST',
            f'{FEISHU}/open-apis/contact/v3/users/batch_get_id?user_id_type=open_id',
            {'emails': emails},
            token=token,
        )
    except Exception as exc:
        print(f'warn: batch_get_id failed: {exc}', file=sys.stderr)
        return {}
    if res.get('code') != 0:
        print(f'warn: batch_get_id code={res.get("code")} msg={res.get("msg")}', file=sys.stderr)
        return {}
    out: dict[str, str] = {}
    for u in (res.get('data', {}) or {}).get('user_list', []) or []:
        if u.get('email') and u.get('user_id'):
            out[u['email']] = u['user_id']
    return out


def resolve_mentions(
    token: str, chat_id: str, assignees: list[dict], user_map: dict
) -> tuple[list[dict], list[str]]:
    """按 override → email → 群成员名 顺序解析。返回 (resolved, missing_display_names)。

    assignees: [{'account': str, 'name': str, 'email': str}, ...]
    resolved:  [{'account', 'name', 'open_id'}, ...]，保持顺序、去重。
    missing:   未解析的 display name 列表，供卡片 note 区列出。
    """
    resolved: list[dict] = []
    seen_open_id: set[str] = set()

    def _push(account: str, name: str, open_id: str):
        if not open_id or open_id in seen_open_id:
            return
        seen_open_id.add(open_id)
        resolved.append({'account': account, 'name': name, 'open_id': open_id})

    # 1) 人工 override
    pending: list[dict] = []
    for a in assignees:
        entry = user_map.get(a['account']) if user_map else None
        open_id = (entry or {}).get('open_id') or ''
        if open_id and not open_id.startswith('ou_REPLACE'):
            _push(a['account'], (entry or {}).get('name', a['name']), open_id)
        else:
            pending.append(a)

    # 2) email 批量查
    emails = [a['email'] for a in pending if a['email']]
    email_map = batch_get_id_by_email(token, emails) if emails else {}
    still_pending: list[dict] = []
    for a in pending:
        oid = email_map.get(a['email']) if a['email'] else None
        if oid:
            _push(a['account'], a['name'], oid)
        else:
            still_pending.append(a)

    # 3) 群成员中文名匹配（仅当还有未解析时才调）
    if still_pending:
        members = list_group_members(token, chat_id)
        unresolved: list[dict] = []
        for a in still_pending:
            oid = members.get(a['name'])
            if oid:
                _push(a['account'], a['name'], oid)
            else:
                unresolved.append(a)
        still_pending = unresolved

    missing = [a['name'] for a in still_pending]
    return resolved, missing


def send(token: str, chat_id: str, card: dict) -> str:
    res = _http(
        'POST',
        f'{FEISHU}/open-apis/im/v1/messages?receive_id_type=chat_id',
        {
            'receive_id': chat_id,
            'msg_type': 'interactive',
            'content': json.dumps(card, ensure_ascii=False),
        },
        token=token,
    )
    if res.get('code') != 0:
        raise RuntimeError(f'send code={res.get("code")} msg={res.get("msg")}')
    return res['data']['message_id']
