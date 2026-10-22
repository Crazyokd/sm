#!/usr/bin/env python3
"""创建 Standard Robots 的 ONES 需求、缺陷和任务。"""

from __future__ import annotations

import argparse
import html
import json
import random
import re
import string
import sys
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple

from ones_api import TEAM_UUID, OnesError, graphql_request, load_auth, request_json

DEFAULT_PROJECT_NAME = 'SROS开发'
DEFAULT_PROJECT_UUID = '2ni3183ALTfZNzBu'
DEFAULT_SPRINT_UUID = 'XCB4dHYh'
TYPE_LABELS = {
    'requirement': '需求',
    'defect': '缺陷',
    'task': '任务',
}

ISSUE_TYPE_DEFAULTS = {
    'requirement': 'Roy6EvKA',
    # SROS开发 项目下「软件缺陷」(NuSzzA62) 不在 IssueTypeScope，会报
    # NotFound.IssueTypeScope；该项目的缺陷类型实际是「缺陷」(8YtVcwaj)。
    'defect': '8YtVcwaj',
    'task': 'B7aTPV3N',
}

PRIORITY_UUIDS = {
    'p0': 'N3f3Lpa7',
    'p1': 'X4zCjKmD',
    'p2': '4Fox75xJ',
    'p3': '7uf9Egvm',
}

REQUIREMENT_FIELD_VALUES = {
    'aprna2v3': {
        'default': '4brebLj8',
        '优化': 'PTiiHzYC',
        '变更': 'SsMwidaX',
    },
    '65LHQvNj': 'current_user',
    '416pdiVz': 'Qxr2oWw5',
    '4ytkWp5h': 'VqBy8wyP',
    'LTN9JJqe': '无',
    'VvbkFLvF': 'qUJCe1iR',
    'PkScSNWa': '5cW9RBT7',
    'Epka4E6A': {
        'default': '274UHSTA',
        '优化': 'B1suru9P',
        '文档': 'QYjsWUda',
    },
    'AJc4frqJ': 'generated_desc',
    'SgUUtEB6': '8eDQQRhu',
}


def normalize_type(value: Optional[str], summary: str) -> str:
    if value:
        text = value.strip().lower()
    else:
        text = summary.lower()
    if any(token in text for token in ('缺陷', 'bug', 'defect', '修复')):
        return 'defect'
    if any(token in text for token in ('任务', 'task', 'todo', '办')):
        return 'task'
    return 'requirement'


def priority_from_text(value: Optional[str], summary: str) -> str:
    text = (value or summary).lower()
    if any(token in text for token in ('p0', '紧急', '严重', '最高')):
        return PRIORITY_UUIDS['p0']
    if any(token in text for token in ('p1', '重要', '较高')):
        return PRIORITY_UUIDS['p1']
    if any(token in text for token in ('p3', '较低', '低优')):
        return PRIORITY_UUIDS['p3']
    return PRIORITY_UUIDS['p2']


def graphql_data(payload: Any) -> Dict[str, Any]:
    if not isinstance(payload, dict):
        raise OnesError(f'GraphQL 返回格式异常：{payload}')
    if isinstance(payload.get('data'), dict):
        return payload['data']
    return payload


def parse_repo_version(repo_root: Path) -> str:
    cmake_file = repo_root / 'CMakeLists.txt'
    content = cmake_file.read_text()
    major = re.search(r'set\(SROS_MAJOR_VERSION\s+(\d+)\)', content)
    minor = re.search(r'set\(SROS_MINOR_VERSION\s+(\d+)\)', content)
    patch = re.search(r'set\(SROS_PATCH_VERSION\s+(\d+)\)', content)
    if not (major and minor and patch):
        raise OnesError(f'无法从 {cmake_file} 解析 SROS 版本号')
    return f'v{major.group(1)}.{minor.group(1)}.{patch.group(1)}'


def find_project(project_name: Optional[str], project_uuid: Optional[str]) -> Tuple[str, str]:
    if project_uuid:
        status, payload = graphql_request('{ projects { uuid name } }')
        if status != 200:
            raise OnesError(f'查询项目失败：HTTP {status} {payload}')
        projects = graphql_data(payload).get('projects', [])
        for project in projects:
            if project.get('uuid') == project_uuid:
                return project_uuid, project.get('name', project_uuid)
        raise OnesError(f'找不到项目 UUID：{project_uuid}')

    if not project_name:
        return DEFAULT_PROJECT_UUID, DEFAULT_PROJECT_NAME

    status, payload = graphql_request('{ projects { uuid name } }')
    if status != 200:
        raise OnesError(f'查询项目失败：HTTP {status} {payload}')
    projects = graphql_data(payload).get('projects', [])
    matches = []
    target = project_name.casefold()
    for project in projects:
        name = (project.get('name') or '').casefold()
        if target == name or target in name:
            matches.append(project)
    if not matches:
        raise OnesError(f'没有匹配到 ONES 项目：{project_name}')
    if len(matches) > 1:
        names = ', '.join(project.get('name', '') for project in matches[:5])
        raise OnesError(f'项目名不唯一：{project_name}。匹配结果：{names}')
    project = matches[0]
    return project['uuid'], project.get('name', project['uuid'])


def issue_type_uuid(issue_type: str, project_uuid: str) -> str:
    if project_uuid == DEFAULT_PROJECT_UUID:
        return ISSUE_TYPE_DEFAULTS[issue_type]
    query = '{ project(uuid: "%s") { issueTypes { uuid name } } }' % project_uuid
    status, payload = graphql_request(query)
    if status != 200:
        raise OnesError(f'查询工作项类型失败：HTTP {status} {payload}')
    issue_types = graphql_data(payload).get('project', {}).get('issueTypes', [])
    keywords = {
        'requirement': ('需求', 'requirement', 'feature'),
        'defect': ('缺陷', 'defect', 'bug'),
        'task': ('任务', 'task', 'todo'),
    }[issue_type]
    for item in issue_types:
        name = (item.get('name') or '').lower()
        if any(keyword in name for keyword in keywords):
            return item['uuid']
    raise OnesError(f'在项目 {project_uuid} 中无法解析工作项类型：{issue_type}')


def find_sprint_uuid(project_uuid: str, repo_root: Path) -> str:
    version = parse_repo_version(repo_root)
    query = (
        '{ sprints(filter: { project_in: ["%s"] }, limit: 10, orderBy: { createTime: DESC }) { uuid name } }'
        % project_uuid
    )
    status, payload = graphql_request(query)
    if status != 200:
        raise OnesError(f'查询迭代失败：HTTP {status} {payload}')
    sprints = graphql_data(payload).get('sprints', [])
    for sprint in sprints:
        name = sprint.get('name') or ''
        if version == name or version in name:
            return sprint['uuid']
    return DEFAULT_SPRINT_UUID


def task_fields() -> Dict[str, Dict[str, Any]]:
    status, payload = request_json('GET', f'/team/{TEAM_UUID}/fields?pool=task')
    if status != 200 or not isinstance(payload, dict):
        raise OnesError(f'查询字段定义失败：HTTP {status} {payload}')
    fields = payload.get('fields', [])
    return {
        field['uuid']: field for field in fields if isinstance(field, dict) and field.get('uuid')
    }


def normalize_option_value(option: Any) -> Any:
    if isinstance(option, dict):
        for key in ('uuid', 'value', 'key', 'name'):
            if option.get(key) is not None:
                return option[key]
    return option


def default_field_value(field: Dict[str, Any], user_uuid: str) -> Any:
    default_value = field.get('default_value')
    if default_value not in (None, '', []):
        return default_value
    field_type = field.get('type')
    if field_type == 8:
        return user_uuid
    options = field.get('options') or []
    if options:
        return normalize_option_value(options[0])
    if field_type in (2, 5, 20):
        return '无'
    if field_type in (1, 4):
        return 0
    return None


def desc_rich_from_text(text: str) -> str:
    paragraphs = [segment.strip() for segment in text.splitlines() if segment.strip()]
    if not paragraphs:
        paragraphs = ['无']
    return ''.join(f'<p>{html.escape(segment)}</p>' for segment in paragraphs)


def requirement_field_values(
    summary: str,
    desc_text: str,
    fields_by_uuid: Dict[str, Dict[str, Any]],
    user_uuid: str,
) -> List[Dict[str, Any]]:
    values: List[Dict[str, Any]] = []
    for field_uuid, raw_value in REQUIREMENT_FIELD_VALUES.items():
        if raw_value == 'current_user':
            value = user_uuid
        elif raw_value == 'generated_desc':
            value = desc_text
        elif isinstance(raw_value, dict):
            value = raw_value['default']
            for token, candidate in raw_value.items():
                if token != 'default' and token in summary:
                    value = candidate
                    break
        else:
            value = raw_value
        values.append({'field_uuid': field_uuid, 'value': value})

    category_field = fields_by_uuid.get('F1BfcE3e')
    if category_field:
        values.append(
            {
                'field_uuid': 'F1BfcE3e',
                'value': default_field_value(category_field, user_uuid),
            }
        )
    return values


def parse_missing_field_uuid(payload: Any) -> Optional[str]:
    if not isinstance(payload, dict):
        return None
    for bad_task in payload.get('bad_tasks', []) or []:
        errcode = bad_task.get('errcode', '')
        marker = 'MissingParameter.Task.'
        if marker in errcode:
            return errcode.split(marker, 1)[1]
        field = bad_task.get('field')
        if isinstance(field, str) and len(field) == 8:
            return field
    return None


def create_uuid(user_uuid: str) -> str:
    return user_uuid[:8] + ''.join(random.choices(string.ascii_letters + string.digits, k=8))


def build_base_task(
    summary: str,
    issue_type: str,
    project_uuid: str,
    sprint_uuid: str,
    desc_rich: str,
    priority_uuid: str,
    user_uuid: str,
) -> Dict[str, Any]:
    return {
        'uuid': create_uuid(user_uuid),
        'owner': user_uuid,
        'assign': user_uuid,
        'summary': summary,
        'parent_uuid': '',
        'sprint_uuid': sprint_uuid,
        'project_uuid': project_uuid,
        'issue_type_uuid': issue_type_uuid(issue_type, project_uuid),
        'desc_rich': desc_rich,
        'priority': priority_uuid,
        'field_values': [],
    }


def merge_field_value(entries: List[Dict[str, Any]], field_uuid: str, value: Any) -> None:
    for entry in entries:
        if entry.get('field_uuid') == field_uuid:
            entry['value'] = value
            return
    entries.append({'field_uuid': field_uuid, 'value': value})


def fetch_created_task(task_uuid: str) -> Dict[str, Any]:
    status, payload = request_json('GET', f'/team/{TEAM_UUID}/task/{task_uuid}/info')
    if status != 200 or not isinstance(payload, dict):
        raise OnesError(f'创建后回查工单详情失败：HTTP {status} {payload}')
    return payload


def create_task(args: argparse.Namespace) -> Dict[str, Any]:
    user_uuid, _ = load_auth()
    issue_type = normalize_type(args.type, args.summary)
    project_uuid, project_name = find_project(args.project, args.project_uuid)
    repo_root = Path(args.repo_root).resolve()
    sprint_uuid = find_sprint_uuid(project_uuid, repo_root)
    fields_by_uuid = task_fields()
    desc_text = args.desc or args.summary
    task = build_base_task(
        summary=args.summary,
        issue_type=issue_type,
        project_uuid=project_uuid,
        sprint_uuid=sprint_uuid,
        desc_rich=desc_rich_from_text(desc_text),
        priority_uuid=priority_from_text(args.priority, args.summary),
        user_uuid=user_uuid,
    )

    if issue_type == 'requirement':
        task['field_values'] = requirement_field_values(
            args.summary, desc_text, fields_by_uuid, user_uuid
        )
    else:
        task['field_values'] = [{'field_uuid': 'HiRJZNR7', 'value': user_uuid}]

    # defect 在 SROS开发 项目下必填字段较多（每填一个再发一次重试），15 轮兜底。
    for _ in range(15):
        status, payload = request_json('POST', f'/team/{TEAM_UUID}/tasks/add2', {'tasks': [task]})
        if (
            status == 200
            and isinstance(payload, dict)
            and payload.get('tasks')
            and not payload.get('bad_tasks')
        ):
            created = fetch_created_task(task['uuid'])
            created['_project_name'] = project_name
            created['_issue_type_key'] = issue_type
            return created

        missing_field_uuid = parse_missing_field_uuid(payload)
        if missing_field_uuid == 'ParentTask':
            task['parent_uuid'] = ''
            continue
        if missing_field_uuid:
            field = fields_by_uuid.get(missing_field_uuid)
            if not field:
                raise OnesError(f'创建失败，出现未知必填字段 {missing_field_uuid}：{payload}')
            merge_field_value(
                task['field_values'], missing_field_uuid, default_field_value(field, user_uuid)
            )
            task['uuid'] = create_uuid(user_uuid)
            continue
        raise OnesError(f'创建失败：HTTP {status} {payload}')

    raise OnesError('创建重试次数过多，仍未成功。')


def print_result(task: Dict[str, Any], as_json: bool) -> None:
    if as_json:
        print(json.dumps(task, ensure_ascii=False, indent=2))
        return
    number = task.get('number')
    summary = task.get('summary')
    project_name = task.get('_project_name', DEFAULT_PROJECT_NAME)
    task_uuid = task.get('uuid')
    issue_type = TYPE_LABELS.get(task.get('_issue_type_key', 'task'), '任务')
    print(f'已创建 {issue_type} #{number}：{summary}')
    print(f'项目：{project_name}')
    print(
        f'链接：https://ones.standard-robots.com:10120/project/#/team/{TEAM_UUID}/task/{task_uuid}'
    )


def main() -> int:
    parser = argparse.ArgumentParser(description='创建 Standard Robots 的 ONES 工作项')
    parser.add_argument('summary', help='工单标题')
    parser.add_argument('--type', help='requirement、defect 或 task')
    parser.add_argument('--project', help='项目名称')
    parser.add_argument('--project-uuid', help='项目 UUID')
    parser.add_argument('--priority', help='p0、p1、p2 或 p3')
    parser.add_argument('--desc', help='描述文本')
    parser.add_argument('--repo-root', default='.', help='用于推断当前迭代的仓库根目录')
    parser.add_argument('--json', action='store_true', help='输出原始 JSON')
    args = parser.parse_args()

    try:
        created = create_task(args)
        print_result(created, as_json=args.json)
        return 0
    except OnesError as exc:
        print(str(exc), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
