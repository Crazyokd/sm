#!/usr/bin/env python3
"""列 ONES 项目下的"发布"工作项, 并可选展开其 publish_content 关联清单。

为什么需要这个脚本:
- ONES root `activityReleases` 查询恒返回 5 条全局发布, 没有真正的分页/项目过滤;
  `project(key) { activityReleases }` 在当前 schema 里不支持。
- 现实数据模型: "发布"在 ONES 里**就是 task**, 用 `issueTypeScope_uuid` 区分。
  SROS 项目的"发布" scope = `XC4uLZFY`。本脚本封装这条 GraphQL 路径并把客户端
  过滤 (status / name 子串) 做好。

典型用法:

    # 列 SROS 项目所有 "未发布 + 必须完成项" 发布 (autopilot_fix_release_must_complete 入口)
    python3 scripts/ones_list_releases.py

    # 同时展开每个发布的 publish_content (关联缺陷/任务清单)
    python3 scripts/ones_list_releases.py --include-content

    # 自定义过滤
    python3 scripts/ones_list_releases.py --status "未发布" --name-contains "必须完成项"
    python3 scripts/ones_list_releases.py --project-uuid 2ni3183ALTfZNzBu --status "已发布"

    # 仅取一条已知发布的 publish_content (跳过列表查询)
    python3 scripts/ones_list_releases.py --release-uuid Uea8vXdNNvZUAc97 --include-content

输出 schema (JSON):

    {
      "releases": [
        {
          "uuid": "...",
          "number": 148851,
          "name": "[待发布]SROS_5.66.x 必须完成项",
          "status_uuid": "...",
          "status_name": "未发布",
          "issue_type_uuid": "...",
          "publish_content_count": 190,            // 仅 --include-content
          "publish_content_done_count": 158,        // 仅 --include-content
          "publish_content": [                      // 仅 --include-content
            {"uuid": "...", "number": ..., "summary": "...",
             "issue_type_uuid": "...", "status_uuid": "...",
             "owner": "...", "assign": "...", "project_uuid": "..."},
            ...
          ]
        }
      ]
    }

退出码: 0 成功 / 1 参数错 / 2 ONES 调用失败。
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
ONES_API = SCRIPT_DIR / 'ones_api.py'

# SROS 项目常量。其它项目自行传 --project-uuid。
SROS_PROJECT_UUID = '2ni3183ALTfZNzBu'
# "发布"工作项类型 scope (SROS 项目专用)。
RELEASE_ISSUE_TYPE_SCOPE = 'XC4uLZFY'
SENSITIVE_ENV_KEYS = (
    'ONES_PASSWORD',
    'ONES_TOKEN',
    'ONES_AUTH_TOKEN',
    'ONES_TOKEN_JSON',
)


def _sensitive_values() -> list[str]:
    values: set[str] = set()
    for key in SENSITIVE_ENV_KEYS:
        value = os.environ.get(key)
        if value:
            values.add(value)

    token_json = os.environ.get('ONES_TOKEN_JSON')
    if token_json:
        try:
            payload = json.loads(token_json)
        except json.JSONDecodeError:
            payload = None
        if isinstance(payload, dict):
            for key in ('token', 'auth_token'):
                value = payload.get(key)
                if isinstance(value, str) and value:
                    values.add(value)

    return sorted(values, key=len, reverse=True)


def _sanitize_output(text: str) -> str:
    sanitized = text
    for value in _sensitive_values():
        sanitized = sanitized.replace(value, '<redacted>')
    return sanitized


def _format_output(label: str, text: str) -> str | None:
    snippet = _sanitize_output(text or '').strip()
    if not snippet:
        return None
    return f'{label}={snippet[:300]}'


def _run_ones(cmd: list[str]) -> dict:
    """调 ones_api.py 并返回 JSON, 失败抛 RuntimeError。"""
    result = subprocess.run(
        ['python3', str(ONES_API), *cmd],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if result.returncode != 0:
        details = [f'exit={result.returncode}']
        for item in (
            _format_output('stdout', result.stdout),
            _format_output('stderr', result.stderr),
        ):
            if item:
                details.append(item)
        raise RuntimeError(f'ones_api {cmd[:2]} ' + '; '.join(details))
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(
            f'ones_api 返回非 JSON: {_sanitize_output(result.stdout)[:300]}'
        ) from exc


def list_releases(
    project_uuid: str,
    issue_type_scope: str,
    limit: int = 5000,
) -> list[dict]:
    """列项目下所有 issueTypeScope=发布 的 task。"""
    query = (
        '{ tasks(filter: {project_in: ["'
        + project_uuid
        + '"], issueTypeScope_in: ["'
        + issue_type_scope
        + '"]}, limit: '
        + str(limit)
        + ') { uuid number name status { uuid name } issueType { uuid name } } }'
    )
    res = _run_ones(['graphql', '--compact', query])
    if 'errors' in res and res['errors']:
        raise RuntimeError(f'GraphQL errors: {res["errors"]}')
    return res.get('data', {}).get('tasks', []) or []


def get_release_detail(release_uuid: str) -> dict:
    """取单个 release task 的完整 info (含 publish_content)。"""
    res = _run_ones(['request', 'GET', f'/team/UNrQ5Ny5/task/{release_uuid}/info'])
    if 'raw' in res and res.get('raw') in ('Not Found', 'Unauthorized'):
        raise RuntimeError(f'task {release_uuid} 取不到: {res["raw"]}')
    return res


def main() -> int:
    parser = argparse.ArgumentParser(
        description='列 ONES 项目下"发布"类工作项, 含 publish_content 关联清单。',
    )
    parser.add_argument(
        '--project-uuid',
        default=SROS_PROJECT_UUID,
        help=f'ONES 项目 UUID, 默认 SROS开发 ({SROS_PROJECT_UUID})',
    )
    parser.add_argument(
        '--issue-type-scope',
        default=RELEASE_ISSUE_TYPE_SCOPE,
        help=f'"发布"工作项 issueTypeScope UUID, 默认 SROS 的 {RELEASE_ISSUE_TYPE_SCOPE}',
    )
    parser.add_argument(
        '--status',
        action='append',
        default=None,
        help='只保留 status.name 命中的发布; 可重复 (默认: 未发布). 传 "*" 取消该过滤',
    )
    parser.add_argument(
        '--name-contains',
        default='必须完成项',
        help='只保留 name 含此子串的发布 (默认: 必须完成项). 传空字符串取消',
    )
    parser.add_argument(
        '--release-uuid',
        help='指定单个 release task UUID; 设置后跳过列表查询, 仅取该条详情',
    )
    parser.add_argument(
        '--include-content',
        action='store_true',
        help='为每条发布展开 publish_content 关联清单 (会对每条多调一次 ONES API)',
    )
    parser.add_argument(
        '--limit',
        type=int,
        default=5000,
        help='GraphQL 列表查询 limit, 默认 5000',
    )
    parser.add_argument(
        '--output',
        choices=('json', 'table'),
        default='json',
        help='输出格式, 默认 json',
    )
    args = parser.parse_args()

    status_filter = args.status or ['未发布']
    if status_filter == ['*']:
        status_filter = None
    name_contains = args.name_contains or None

    if not ONES_API.exists():
        print(f'找不到 ones_api.py: {ONES_API}', file=sys.stderr)
        return 1

    try:
        if args.release_uuid:
            detail = get_release_detail(args.release_uuid)
            releases = [
                {
                    'uuid': detail.get('uuid'),
                    'number': detail.get('number'),
                    'name': detail.get('summary'),
                    'status_uuid': detail.get('status_uuid'),
                    'status_name': None,
                    'issue_type_uuid': detail.get('issue_type_uuid'),
                }
            ]
        else:
            raw = list_releases(args.project_uuid, args.issue_type_scope, args.limit)
            releases = []
            for t in raw:
                name = t.get('name') or ''
                status = (t.get('status') or {}).get('name') or ''
                if status_filter and status not in status_filter:
                    continue
                if name_contains and name_contains not in name:
                    continue
                releases.append(
                    {
                        'uuid': t.get('uuid'),
                        'number': t.get('number'),
                        'name': name,
                        'status_uuid': (t.get('status') or {}).get('uuid'),
                        'status_name': status,
                        'issue_type_uuid': (t.get('issueType') or {}).get('uuid'),
                    }
                )
    except RuntimeError as exc:
        print(f'failed: {exc}', file=sys.stderr)
        return 2

    if args.include_content:
        for r in releases:
            try:
                detail = get_release_detail(r['uuid'])
            except RuntimeError as exc:
                print(f'warn: release {r["uuid"]} 取 detail 失败: {exc}', file=sys.stderr)
                continue
            r['publish_content_count'] = detail.get('publish_content_count')
            r['publish_content_done_count'] = detail.get('publish_content_done_count')
            r['publish_content'] = detail.get('publish_content') or []

    payload = {'releases': releases}

    if args.output == 'json':
        json.dump(payload, sys.stdout, ensure_ascii=False, indent=2)
        sys.stdout.write('\n')
    else:
        print(f'{"NUMBER":>8s}  {"STATUS":<10s}  {"CONTENT":>8s}  NAME')
        for r in releases:
            cnt = r.get('publish_content_count')
            done = r.get('publish_content_done_count')
            cnt_str = f'{done}/{cnt}' if cnt is not None else '-'
            print(
                f'{r.get("number", "-"):>8}  {r.get("status_name", "-"):<10s}  '
                f'{cnt_str:>8s}  {r.get("name", "-")}'
            )
    return 0


if __name__ == '__main__':
    sys.exit(main())
