#!/usr/bin/env python3
"""Standard Robots ONES API 辅助脚本。"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

SCRIPT_DIR = Path(__file__).resolve().parent
REFRESH_SCRIPT = SCRIPT_DIR / 'ones_refresh_token.py'

ONES_HOST = 'https://ones.standard-robots.com:10120'
API_BASE = f'{ONES_HOST}/project/api/project'
TEAM_UUID = 'UNrQ5Ny5'
GRAPHQL_ENDPOINT = f'{API_BASE}/team/{TEAM_UUID}/items/graphql'
TOKEN_FILE = Path.home() / '.ones_token'


class OnesError(RuntimeError):
    """ONES 认证或请求失败时抛出。"""


def _parse_json_text(raw: str) -> Any:
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return {'raw': raw}


def _read_json_arg(value: Optional[str]) -> Optional[Any]:
    if value is None:
        return None
    if value.startswith('@'):
        return json.loads(Path(value[1:]).read_text())
    return json.loads(value)


def _auth_from_env() -> Optional[Tuple[str, str]]:
    user_id = os.environ.get('ONES_USER_ID') or os.environ.get('ONES_UUID')
    token = os.environ.get('ONES_AUTH_TOKEN') or os.environ.get('ONES_TOKEN')
    if user_id and token:
        return user_id, token
    token_json = os.environ.get('ONES_TOKEN_JSON')
    if token_json:
        try:
            payload = json.loads(token_json)
        except json.JSONDecodeError:
            return None
        if not isinstance(payload, dict):
            return None
        user_id = payload.get('uuid')
        token = payload.get('token')
        if user_id and token:
            return user_id, token
    return None


def _auth_from_file() -> Optional[Tuple[str, str]]:
    if not TOKEN_FILE.exists():
        return None
    try:
        payload = json.loads(TOKEN_FILE.read_text())
    except (OSError, json.JSONDecodeError):
        return None
    user_id = payload.get('uuid')
    token = payload.get('token')
    if user_id and token:
        return user_id, token
    return None


def _write_token(user_id: str, token: str) -> None:
    TOKEN_FILE.write_text(json.dumps({'uuid': user_id, 'token': token}, ensure_ascii=False))
    os.chmod(TOKEN_FILE, 0o600)


def login_from_password() -> Tuple[str, str]:
    email = os.environ.get('ONES_EMAIL')
    password = os.environ.get('ONES_PASSWORD')
    if not email or not password:
        raise OnesError('未设置 ONES_EMAIL / ONES_PASSWORD，跳过密码登录。')
    status, payload = raw_request(
        'POST',
        f'{API_BASE}/auth/login',
        {'email': email, 'password': password},
        auth=None,
        retry_on_auth=False,
    )
    user = payload.get('user', {}) if isinstance(payload, dict) else {}
    user_id = user.get('uuid')
    token = user.get('token')
    if status != 200 or not user_id or not token:
        raise OnesError(f'ONES 登录失败：HTTP {status} {payload}')
    _write_token(user_id, token)
    return user_id, token


def refresh_from_browser() -> Tuple[str, str]:
    """通过 ones_refresh_token.py 从持久化 Chromium profile 抽取 cookie。"""
    if not REFRESH_SCRIPT.exists():
        raise OnesError(f'找不到 {REFRESH_SCRIPT}')
    env = os.environ.copy()
    env.setdefault('HEADLESS', '1')
    proc = subprocess.run(
        [sys.executable, str(REFRESH_SCRIPT)],
        env=env,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise OnesError(
            f'浏览器刷新失败 (exit {proc.returncode}): {proc.stderr.strip() or proc.stdout.strip()}'
        )
    file_auth = _auth_from_file()
    if not file_auth:
        raise OnesError('浏览器刷新已退出但 ~/.ones_token 仍无效')
    return file_auth


def load_auth(force_refresh: bool = False) -> Tuple[str, str]:
    env_auth = _auth_from_env()
    if env_auth and not force_refresh:
        return env_auth
    if not force_refresh:
        file_auth = _auth_from_file()
        if file_auth:
            return file_auth

    # force_refresh 路径：环境变量仍优先（CI 直接传入新 token 时不要触发交互）
    if env_auth and not (os.environ.get('ONES_EMAIL') and os.environ.get('ONES_PASSWORD')):
        return env_auth

    # 1) 有 email/password 时走密码登录（CI / 服务器无 GUI 场景）
    if os.environ.get('ONES_EMAIL') and os.environ.get('ONES_PASSWORD'):
        return login_from_password()

    # 2) 默认走浏览器 SSO 刷新（飞书扫码 / 复用持久化 profile）
    try:
        return refresh_from_browser()
    except OnesError as exc:
        raise OnesError(
            f'{exc}\n\n请任选其一恢复认证：\n'
            '  - 终端执行 `python3 .agents/skills/ones/scripts/ones_refresh_token.py` 完成扫码\n'
            '  - 或设置 ONES_USER_ID/ONES_AUTH_TOKEN 环境变量\n'
            '  - 或设置 ONES_EMAIL/ONES_PASSWORD'
        )


def build_headers(auth: Optional[Tuple[str, str]]) -> Dict[str, str]:
    headers = {
        'Content-Type': 'application/json',
        'Referer': ONES_HOST,
    }
    if auth:
        user_id, token = auth
        headers['Ones-User-Id'] = user_id
        headers['Ones-Auth-Token'] = token
    return headers


def normalize_url(path: str) -> str:
    if path.startswith('http://') or path.startswith('https://'):
        return path
    if path.startswith('/'):
        return f'{API_BASE}{path}'
    return f'{API_BASE}/{path}'


def raw_request(
    method: str,
    path: str,
    data: Optional[Any] = None,
    auth: Optional[Tuple[str, str]] = None,
    retry_on_auth: bool = True,
) -> Tuple[int, Any]:
    body = None
    if data is not None:
        body = json.dumps(data, ensure_ascii=False).encode('utf-8')
    request = urllib.request.Request(
        normalize_url(path),
        data=body,
        headers=build_headers(auth),
        method=method.upper(),
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read().decode('utf-8')
            return response.getcode(), _parse_json_text(raw)
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode('utf-8', 'replace')
        payload = _parse_json_text(raw)
        if exc.code in (401, 403) and retry_on_auth and auth is not None:
            refreshed = load_auth(force_refresh=True)
            return raw_request(method, path, data=data, auth=refreshed, retry_on_auth=False)
        return exc.code, payload


def request_json(method: str, path: str, data: Optional[Any] = None) -> Tuple[int, Any]:
    auth = load_auth()
    return raw_request(method, path, data=data, auth=auth)


def graphql_request(query: str, variables: Optional[Any] = None) -> Tuple[int, Any]:
    payload: Dict[str, Any] = {'query': query}
    if variables is not None:
        payload['variables'] = variables
    auth = load_auth()
    return raw_request('POST', GRAPHQL_ENDPOINT, data=payload, auth=auth)


def _print_payload(payload: Any, compact: bool) -> None:
    if compact:
        print(json.dumps(payload, ensure_ascii=False, separators=(',', ':')))
        return
    print(json.dumps(payload, ensure_ascii=False, indent=2))


def main() -> int:
    parser = argparse.ArgumentParser(description='ONES API 辅助脚本')
    subparsers = parser.add_subparsers(dest='command', required=True)

    login_parser = subparsers.add_parser('login', help='校验认证，并在可能时缓存 token')
    login_parser.add_argument(
        '--print', action='store_true', dest='print_auth', help='输出认证 JSON'
    )

    request_parser = subparsers.add_parser('request', help='发起 ONES API 请求')
    request_parser.add_argument('method', help='HTTP 方法')
    request_parser.add_argument('path', help='绝对 URL，或相对 API_BASE 的路径')
    request_parser.add_argument('--data', help='内联 JSON，或 @path/to/file.json')
    request_parser.add_argument('--compact', action='store_true', help='输出紧凑 JSON')

    graphql_parser = subparsers.add_parser('graphql', help='发起 ONES GraphQL 请求')
    graphql_parser.add_argument('query', help='GraphQL 查询语句')
    graphql_parser.add_argument('--variables', help='内联 JSON，或 @path/to/file.json')
    graphql_parser.add_argument('--compact', action='store_true', help='输出紧凑 JSON')

    args = parser.parse_args()

    try:
        if args.command == 'login':
            user_id, token = load_auth()
            if args.print_auth:
                _print_payload({'uuid': user_id, 'token': token}, compact=False)
            else:
                print('ONES 认证已就绪。')
            return 0

        if args.command == 'request':
            data = _read_json_arg(args.data)
            status, payload = request_json(args.method, args.path, data=data)
            _print_payload(payload, compact=args.compact)
            return 0 if 200 <= status < 300 else 1

        if args.command == 'graphql':
            variables = _read_json_arg(args.variables)
            status, payload = graphql_request(args.query, variables=variables)
            _print_payload(payload, compact=args.compact)
            return 0 if 200 <= status < 300 else 1
    except OnesError as exc:
        print(str(exc), file=sys.stderr)
        return 2

    return 1


if __name__ == '__main__':
    sys.exit(main())
