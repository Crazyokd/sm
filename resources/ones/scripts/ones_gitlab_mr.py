#!/usr/bin/env python3
"""GitLab MR helper exposed by the ONES skill.

This script intentionally lives in the ONES skill so callers can keep a
single workflow entry point while GitLab permissions come from local
developer credentials.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

try:
    import gitlab
except ImportError:  # pragma: no cover - depends on host image
    gitlab = None


DEFAULT_GITLAB_URL = 'http://git.standard-robots.com'
DEFAULT_PROJECT_ID = '38'


class GitLabMrError(RuntimeError):
    """GitLab helper failure."""


def print_json(payload: Any, compact: bool = False) -> None:
    if compact:
        print(json.dumps(payload, ensure_ascii=False, separators=(',', ':')))
        return
    print(json.dumps(payload, ensure_ascii=False, indent=2))


def load_token() -> Tuple[Optional[str], Optional[str]]:
    for env_name in ('SROS_BOT', 'GITLAB_PRIVATE_TOKEN', 'GITLAB_TOKEN'):
        token = os.environ.get(env_name)
        if token:
            return token, env_name
    return None, None


def git_output(args: Sequence[str], cwd: Path) -> Optional[str]:
    try:
        result = subprocess.run(
            ['git', *args],
            cwd=str(cwd),
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return result.stdout.strip()


def parse_remote_project_path(remote_url: str) -> Optional[str]:
    if not remote_url:
        return None
    remote_url = remote_url.strip()
    patterns = (
        r'git@[^:]+:(?P<path>.+?)(?:\.git)?$',
        r'https?://[^/]+/(?P<path>.+?)(?:\.git)?$',
        r'ssh://git@[^/]+/(?P<path>.+?)(?:\.git)?$',
    )
    for pattern in patterns:
        match = re.match(pattern, remote_url)
        if match:
            return match.group('path')
    return None


def resolve_project_id(repo_root: Path, explicit_project: Optional[str]) -> str:
    if explicit_project:
        return explicit_project
    env_project = os.environ.get('GITLAB_PROJECT_ID') or os.environ.get('CI_PROJECT_ID')
    if env_project:
        return env_project
    remote_url = git_output(['remote', 'get-url', 'origin'], repo_root)
    project_path = parse_remote_project_path(remote_url or '')
    if project_path:
        return project_path
    return DEFAULT_PROJECT_ID


def connect(repo_root: Path, project: Optional[str]) -> Tuple[Any, Any, Dict[str, Any]]:
    if gitlab is None:
        raise GitLabMrError('缺少 python-gitlab，请在当前环境安装 python-gitlab')
    token, token_source = load_token()
    if not token:
        raise GitLabMrError(
            '缺少本地 GitLab 凭证：请设置 SROS_BOT/GITLAB_PRIVATE_TOKEN/GITLAB_TOKEN'
        )
    url = os.environ.get('GITLAB_URL', DEFAULT_GITLAB_URL).rstrip('/')
    project_id = resolve_project_id(repo_root, project)
    gl = gitlab.Gitlab(url, private_token=token)
    gl.auth()
    gitlab_project = gl.projects.get(project_id)
    meta = {
        'gitlab_url': url,
        'project': project_id,
        'project_web_url': getattr(gitlab_project, 'web_url', None),
        'token_source': token_source,
    }
    return gl, gitlab_project, meta


def mr_to_dict(mr: Any) -> Dict[str, Any]:
    return {
        'iid': getattr(mr, 'iid', None),
        'id': getattr(mr, 'id', None),
        'title': getattr(mr, 'title', None),
        'state': getattr(mr, 'state', None),
        'draft': bool(getattr(mr, 'draft', False) or getattr(mr, 'work_in_progress', False)),
        'source_branch': getattr(mr, 'source_branch', None),
        'target_branch': getattr(mr, 'target_branch', None),
        'web_url': getattr(mr, 'web_url', None),
        'merge_commit_sha': getattr(mr, 'merge_commit_sha', None),
        'squash_commit_sha': getattr(mr, 'squash_commit_sha', None),
        'sha': getattr(mr, 'sha', None),
        'updated_at': getattr(mr, 'updated_at', None),
        'created_at': getattr(mr, 'created_at', None),
        'author': (getattr(mr, 'author', {}) or {}).get('username')
        if isinstance(getattr(mr, 'author', None), dict)
        else getattr(mr, 'author', None),
        'assignees': [
            item.get('username') or item.get('name')
            for item in (getattr(mr, 'assignees', None) or [])
            if isinstance(item, dict)
        ],
    }


def read_text_arg(value: str) -> str:
    """`@path` 读取文件内容；否则按字面量返回。"""
    if value.startswith('@'):
        return Path(value[1:]).read_text(encoding='utf-8')
    return value


def note_to_dict(note: Any) -> Dict[str, Any]:
    get = note.get if isinstance(note, dict) else (lambda k, d=None: getattr(note, k, d))
    author = get('author')
    if isinstance(author, dict):
        author = author.get('username') or author.get('name')
    return {
        'id': get('id'),
        'author': author,
        'body': get('body'),
        'system': bool(get('system', False)),
        'resolvable': bool(get('resolvable', False)),
        'resolved': bool(get('resolved', False)),
        'created_at': get('created_at'),
        'updated_at': get('updated_at'),
    }


def resolve_user_id(gl: Any, assignee: str) -> Tuple[int, str]:
    """把 username / email / 姓名 解析成 (user_id, username)。0 个或多个匹配则报错。"""
    by_username = list(gl.users.list(username=assignee, get_all=True))
    if len(by_username) == 1:
        user = by_username[0]
        return user.id, user.username
    by_search = list(gl.users.list(search=assignee, get_all=True))
    if len(by_search) == 1:
        user = by_search[0]
        return user.id, user.username
    candidates = by_username or by_search
    if not candidates:
        raise GitLabMrError(f'未找到匹配的 GitLab 用户：{assignee}')
    listed = ', '.join(f'{u.username}(id={u.id})' for u in candidates)
    raise GitLabMrError(f'用户 {assignee} 匹配到多个 GitLab 账号，请改用 --assignee-id：{listed}')


def state_value(mr: Dict[str, Any]) -> str:
    state = (mr.get('state') or '').lower()
    if state == 'merged':
        return 'merged'
    if state == 'opened' and mr.get('draft'):
        return 'draft'
    if state == 'opened':
        return 'opened'
    if state == 'closed':
        return 'closed'
    return state or 'unknown'


def state_rank(mr: Dict[str, Any]) -> int:
    return {
        'merged': 4,
        'opened': 3,
        'draft': 2,
        'closed': 1,
    }.get(state_value(mr), 0)


def list_all_mrs(project: Any, **kwargs: Any) -> List[Any]:
    return list(project.mergerequests.list(all=True, **kwargs))


def filter_mrs(
    mrs: Iterable[Any],
    ones_id: Optional[str],
    source_pattern: Optional[str],
    target_branch: Optional[str],
) -> List[Dict[str, Any]]:
    needle = f'#{ones_id}' if ones_id else None
    result: List[Dict[str, Any]] = []
    for mr in mrs:
        item = mr_to_dict(mr)
        if source_pattern and not fnmatch.fnmatch(item.get('source_branch') or '', source_pattern):
            continue
        if target_branch and item.get('target_branch') != target_branch:
            continue
        if needle:
            haystacks = (
                item.get('title') or '',
                getattr(mr, 'description', None) or '',
                item.get('source_branch') or '',
            )
            if not any(needle in value or ones_id in value for value in haystacks):
                continue
        result.append(item)
    result.sort(key=lambda item: (state_rank(item), item.get('updated_at') or ''), reverse=True)
    return result


def cmd_auth(args: argparse.Namespace) -> int:
    _, project, meta = connect(args.repo_root, args.project)
    print_json({'ok': True, **meta, 'name': getattr(project, 'name', None)}, compact=args.compact)
    return 0


def cmd_whoami(args: argparse.Namespace) -> int:
    gl, _, meta = connect(args.repo_root, args.project)
    user = gl.user
    print_json(
        {
            'ok': True,
            **meta,
            'user': {
                'id': getattr(user, 'id', None),
                'username': getattr(user, 'username', None),
                'name': getattr(user, 'name', None),
                'state': getattr(user, 'state', None),
                'bot': getattr(user, 'bot', None),
            },
        },
        compact=args.compact,
    )
    return 0


def cmd_search(args: argparse.Namespace) -> int:
    _, project, meta = connect(args.repo_root, args.project)
    query_kwargs: Dict[str, Any] = {'state': args.state}
    if args.search:
        query_kwargs['search'] = args.search
    elif args.ones_id:
        query_kwargs['search'] = f'#{args.ones_id}'
    mrs = list_all_mrs(project, **query_kwargs)
    source_pattern = args.source_pattern
    if not source_pattern and args.ones_id:
        source_pattern = f'fix/{args.ones_id}_*'
    items = filter_mrs(mrs, args.ones_id, source_pattern, args.target_branch)
    print_json(
        {
            'ok': True,
            **meta,
            'count': len(items),
            'latest': items[0] if items else None,
            'items': items,
        },
        compact=args.compact,
    )
    return 0


def parse_mr_iid(value: str) -> str:
    match = re.search(r'/merge_requests/(\d+)', value)
    if match:
        return match.group(1)
    return value


def cmd_get(args: argparse.Namespace) -> int:
    _, project, meta = connect(args.repo_root, args.project)
    mr = project.mergerequests.get(parse_mr_iid(args.mr))
    print_json({'ok': True, **meta, 'mr': mr_to_dict(mr)}, compact=args.compact)
    return 0


def cmd_create(args: argparse.Namespace) -> int:
    gl, project, meta = connect(args.repo_root, args.project)
    payload: Dict[str, Any] = {
        'source_branch': args.source_branch,
        'target_branch': args.target_branch,
        'title': args.title,
        'description': args.description,
        'remove_source_branch': args.remove_source_branch,
        'squash': args.squash,
    }
    resolved_from: Optional[Dict[str, Any]] = None
    if args.assignee_id:
        payload['assignee_id'] = args.assignee_id
    elif args.assignee:
        assignee_id, resolved_username = resolve_user_id(gl, args.assignee)
        payload['assignee_id'] = assignee_id
        resolved_from = {'input': args.assignee, 'username': resolved_username}
    mr = project.mergerequests.create(payload)
    print_json(
        {'ok': True, **meta, 'resolved_from': resolved_from, 'mr': mr_to_dict(mr)},
        compact=args.compact,
    )
    return 0


def cmd_notes(args: argparse.Namespace) -> int:
    _, project, meta = connect(args.repo_root, args.project)
    mr = project.mergerequests.get(parse_mr_iid(args.mr))
    discussions: List[Dict[str, Any]] = []
    for disc in mr.discussions.list(all=True):
        raw_notes = disc.attributes.get('notes') or []
        notes = [note_to_dict(item) for item in raw_notes]
        if not args.include_system:
            notes = [item for item in notes if not item['system']]
        if not notes:
            continue
        discussions.append(
            {
                'discussion_id': disc.id,
                'individual_note': bool(disc.attributes.get('individual_note')),
                'notes': notes,
            }
        )
    print_json(
        {
            'ok': True,
            **meta,
            'mr_iid': getattr(mr, 'iid', None),
            'web_url': getattr(mr, 'web_url', None),
            'discussion_count': len(discussions),
            'discussions': discussions,
        },
        compact=args.compact,
    )
    return 0


def cmd_reply(args: argparse.Namespace) -> int:
    _, project, meta = connect(args.repo_root, args.project)
    mr = project.mergerequests.get(parse_mr_iid(args.mr))
    body = read_text_arg(args.body)
    if not body.strip():
        raise GitLabMrError('回复正文为空')
    if args.discussion_id:
        discussion = mr.discussions.get(args.discussion_id)
        note = discussion.notes.create({'body': body})
        result: Dict[str, Any] = {
            'discussion_id': args.discussion_id,
            'note_id': getattr(note, 'id', None),
        }
    else:
        note = mr.notes.create({'body': body})
        result = {'note_id': getattr(note, 'id', None)}
    print_json(
        {
            'ok': True,
            **meta,
            'mr_iid': getattr(mr, 'iid', None),
            'web_url': getattr(mr, 'web_url', None),
            **result,
        },
        compact=args.compact,
    )
    return 0


def cmd_resolve_user(args: argparse.Namespace) -> int:
    gl, _, meta = connect(args.repo_root, args.project)
    try:
        user_id, username = resolve_user_id(gl, args.assignee)
    except GitLabMrError as exc:
        print_json(
            {'ok': False, **meta, 'input': args.assignee, 'error': str(exc)}, compact=args.compact
        )
        return 2
    user = gl.users.get(user_id)
    print_json(
        {
            'ok': True,
            **meta,
            'input': args.assignee,
            'user': {
                'id': user_id,
                'username': username,
                'name': getattr(user, 'name', None),
                'state': getattr(user, 'state', None),
                'bot': getattr(user, 'bot', None),
            },
        },
        compact=args.compact,
    )
    return 0


def cmd_assign(args: argparse.Namespace) -> int:
    gl, project, meta = connect(args.repo_root, args.project)
    mr = project.mergerequests.get(parse_mr_iid(args.mr))
    resolved_from: Optional[Dict[str, Any]] = None
    if args.assignee_id:
        assignee_id = args.assignee_id
    else:
        assignee_id, resolved_username = resolve_user_id(gl, args.assignee)
        resolved_from = {'input': args.assignee, 'username': resolved_username}
    mr.assignee_id = assignee_id
    mr.save()
    mr = project.mergerequests.get(parse_mr_iid(args.mr))
    print_json(
        {
            'ok': True,
            **meta,
            'mr_iid': getattr(mr, 'iid', None),
            'web_url': getattr(mr, 'web_url', None),
            'assignee_id': assignee_id,
            'resolved_from': resolved_from,
            'assignees': mr_to_dict(mr)['assignees'],
        },
        compact=args.compact,
    )
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description='ONES skill 的 GitLab MR 权限桥接脚本')
    parser.add_argument('--repo-root', type=Path, default=Path.cwd(), help='Git 仓库根目录')
    parser.add_argument('--project', help='GitLab project id 或 namespace/path，默认从 origin 解析')
    parser.add_argument('--compact', action='store_true', help='输出紧凑 JSON')
    subparsers = parser.add_subparsers(dest='command', required=True)

    subparsers.add_parser('auth', help='校验 GitLab 凭证和项目权限')

    subparsers.add_parser('whoami', help='输出当前 token 对应的 GitLab 用户')

    resolve_parser = subparsers.add_parser(
        'resolve-user', help='把 username/email/姓名 反查成 GitLab user（不修改任何资源）'
    )
    resolve_parser.add_argument('--assignee', required=True, help='username / email / 姓名')

    search_parser = subparsers.add_parser('search', help='查询关联 MR')
    search_parser.add_argument('--ones-id', help='ONES 数字 ID，例如 150962')
    search_parser.add_argument('--search', help='GitLab search 字符串，默认 #<ones_id>')
    search_parser.add_argument('--source-pattern', help='source branch glob，例如 fix/150962_*')
    search_parser.add_argument('--target-branch', help='过滤 target branch')
    search_parser.add_argument(
        '--state',
        default='all',
        choices=('opened', 'closed', 'locked', 'merged', 'all'),
        help='GitLab MR state',
    )

    get_parser = subparsers.add_parser('get', help='按 MR IID 或 URL 获取 MR')
    get_parser.add_argument('mr', help='MR IID 或 web_url')

    create_parser = subparsers.add_parser('create', help='创建 MR')
    create_parser.add_argument('--source-branch', required=True)
    create_parser.add_argument('--target-branch', required=True)
    create_parser.add_argument('--title', required=True)
    create_parser.add_argument('--description', required=True)
    create_assignee = create_parser.add_mutually_exclusive_group()
    create_assignee.add_argument('--assignee-id', type=int, help='GitLab numeric user id')
    create_assignee.add_argument('--assignee', help='username / email / 姓名，自动反查 numeric id')
    create_parser.add_argument('--remove-source-branch', action='store_true')
    create_parser.add_argument('--squash', action='store_true')

    notes_parser = subparsers.add_parser('notes', help='列出 MR 的 discussion / note')
    notes_parser.add_argument('mr', help='MR IID 或 web_url')
    notes_parser.add_argument(
        '--include-system', action='store_true', help='包含 system note（默认过滤）'
    )

    reply_parser = subparsers.add_parser('reply', help='在 MR discussion 下回复，或发独立 note')
    reply_parser.add_argument('mr', help='MR IID 或 web_url')
    reply_parser.add_argument(
        '--discussion-id', help='回复到指定 discussion 线程；省略则发独立 note'
    )
    reply_parser.add_argument('--body', required=True, help='回复正文；`@path` 形式从文件读取')

    assign_parser = subparsers.add_parser('assign', help='设置已建 MR 的 assignee')
    assign_parser.add_argument('mr', help='MR IID 或 web_url')
    assign_group = assign_parser.add_mutually_exclusive_group(required=True)
    assign_group.add_argument('--assignee-id', type=int, help='GitLab numeric user id')
    assign_group.add_argument('--assignee', help='username / email / 姓名，自动反查 numeric id')

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        if args.command == 'auth':
            return cmd_auth(args)
        if args.command == 'whoami':
            return cmd_whoami(args)
        if args.command == 'resolve-user':
            return cmd_resolve_user(args)
        if args.command == 'search':
            return cmd_search(args)
        if args.command == 'get':
            return cmd_get(args)
        if args.command == 'create':
            return cmd_create(args)
        if args.command == 'notes':
            return cmd_notes(args)
        if args.command == 'reply':
            return cmd_reply(args)
        if args.command == 'assign':
            return cmd_assign(args)
    except GitLabMrError as exc:
        print_json({'ok': False, 'error': str(exc)}, compact=getattr(args, 'compact', False))
        return 2
    except Exception as exc:  # noqa: BLE001 - helper should report structured errors
        print_json(
            {'ok': False, 'error': f'{type(exc).__name__}: {exc}'},
            compact=getattr(args, 'compact', False),
        )
        return 1
    return 1


if __name__ == '__main__':
    sys.exit(main())
