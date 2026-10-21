import json
import os
import tempfile
from collections import defaultdict
from collections.abc import Mapping
from pathlib import Path

import tomlkit

from .catalog import Catalog, UnsupportedResource, contained
from .paths import resolve_scope
from .providers import codex, kimi
from .state import ApplyReport, State


class Conflict(ValueError):
    pass


def plan_apply(catalog: Catalog, sources: dict[str, Path], home: Path, state: State):
    writes = []
    state.planning_results = {}
    for resource in catalog.resources:
        try:
            scope = resolve_scope(resource, catalog, home)
            if resource.scope == "project" and not scope.is_dir():
                raise ValueError(f"project directory missing: {scope}")
            current = []
            for agent, path in resource.targets.items():
                payload = contained(sources[resource.id], path)
                current.extend({"codex": codex, "kimi": kimi}[agent].render(resource, payload, scope))
            writes.extend(w for w in current if w not in writes)
        except (ValueError, KeyError, OSError) as error:
            state.planning_results[resource.id] = {"status": "unsupported" if isinstance(error, UnsupportedResource) else "failed", "message": str(error)}
    return writes


def block(write):
    return f"<!-- SM:{write.resource_id} -->\n{write.content.decode().rstrip()}\n<!-- /SM:{write.resource_id} -->\n"


def merge_entries(document, fragment, remove=False):
    if not isinstance(document, Mapping) or not isinstance(fragment, Mapping):
        raise Conflict("configuration must be an object")
    for section, values in fragment.items():
        if isinstance(values, list):
            target = document.setdefault(section, [])
            if not isinstance(target, list):
                raise Conflict(f"configuration section must be a list: {section}")
            for value in values:
                if remove:
                    if value not in target:
                        raise Conflict("managed hook was edited or removed")
                    target.remove(value)
                else:
                    if value in target:
                        raise Conflict("hook already exists outside this resource")
                    target.append(value)
        elif section == "hooks":
            target = document.setdefault(section, {})
            if not isinstance(target, Mapping) or not isinstance(values, Mapping):
                raise Conflict("hooks configuration must be an object")
            for event, groups in values.items():
                entries = target.setdefault(event, [])
                if not isinstance(entries, list) or not isinstance(groups, list):
                    raise Conflict("hook event entries must be a list")
                for group in groups:
                    if remove:
                        if group not in entries:
                            raise Conflict("managed hook was edited or removed")
                        entries.remove(group)
                    else:
                        if group in entries:
                            raise Conflict("hook already exists outside this resource")
                        entries.append(group)
                if not entries:
                    del target[event]
        else:
            target = document.setdefault(section, {})
            if not isinstance(target, Mapping) or not isinstance(values, Mapping):
                raise Conflict(f"configuration section must be an object: {section}")
            for key, value in values.items():
                if remove:
                    if key not in target or target[key] != value:
                        raise Conflict(f"managed entry was edited: {key}")
                    del target[key]
                else:
                    if key in target:
                        raise Conflict(f"unmanaged or duplicate entry: {key}")
                    target[key] = value


def atomic_write(path, content, executable=None):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=".sm-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        permissions = path.stat().st_mode & 0o777 if path.exists() else 0o600
        if executable is not None:
            permissions = permissions | 0o100 if executable else permissions & ~0o111
        os.chmod(temporary, permissions)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def _prepare(path, new, old):
    if any(p.is_symlink() for p in [path, *path.parents]):
        raise Conflict("target path contains a symlink")
    modes = {w.mode for w in new + old}
    if len(modes) != 1:
        raise Conflict("incompatible writes to the same path")
    mode = modes.pop()
    content = path.read_bytes() if path.exists() else None
    if mode == "file":
        if old and (len(old) != 1 or content != old[0].content):
            raise Conflict("managed file was changed locally")
        if not old and content is not None:
            raise Conflict("unmanaged file already exists")
        if len(new) > 1:
            raise Conflict("multiple resources target the same file")
        result = new[0].content if new else None
    elif mode == "markdown_block":
        text = (content or b"").decode()
        for write in old:
            if text.count(block(write)) != 1:
                raise Conflict("managed rules were changed locally")
            text = text.replace(block(write), "", 1)
        for write in new:
            if f"<!-- SM:{write.resource_id} -->" in text:
                raise Conflict("unmanaged resource marker already exists")
            text += ("\n" if text and not text.endswith("\n") else "") + block(write)
        result = text.encode() if text else None
    else:
        document = (tomlkit.parse((content or b"").decode()) if mode == "toml_entries"
                    else json.loads(content or b"{}"))
        for write in old:
            merge_entries(document, write.content, remove=True)
        for write in new:
            merge_entries(document, write.content)
        result = (tomlkit.dumps(document).encode() if mode == "toml_entries"
                  else (json.dumps(document, ensure_ascii=False, indent=2) + "\n").encode())
    return content, result


def apply_writes(writes, state: State) -> ApplyReport:
    report = ApplyReport(dict(state.planning_results))
    desired, previous = defaultdict(list), defaultdict(list)
    for write in writes:
        if write not in desired[write.path]:
            desired[write.path].append(write)
    for write in state.owned():
        previous[write.path].append(write)
    prepared = {}
    blocked = set(state.planning_results)
    for path in sorted(set(desired) | set(previous)):
        new, old = desired[path], previous[path]
        owners = {w.resource_id for w in new + old}
        try:
            before, after = _prepare(path, new, old)
            prepared[path] = (before, after, new, owners)
        except (ValueError, OSError) as error:
            blocked.update(owners)
            for rid in owners:
                report.results[rid] = {"status": "conflict" if isinstance(error, Conflict) else "failed", "message": str(error)}
    while True:
        expanded = blocked | {rid for _, _, _, owners in prepared.values() if owners & blocked for rid in owners}
        if expanded == blocked:
            break
        blocked = expanded
    for rid in blocked:
        report.results.setdefault(rid, {"status": "conflict", "message": "共享目标的其他受管资源未通过检查"})
    order = sorted(prepared, key=lambda p: (prepared[p][1] is None,
                   any(w.mode != "file" for w in prepared[p][2]), str(p)))
    for path in order:
        before, after, new, owners = prepared[path]
        if owners & blocked:
            continue
        executable = new[0].executable if new and new[0].mode == "file" else None
        try:
            if after is None:
                path.unlink(missing_ok=True)
            elif before != after or (executable is not None and bool(path.stat().st_mode & 0o111) != executable):
                atomic_write(path, after, executable)
            state.replace_owned(path, new)
            for rid in owners:
                report.results[rid] = {"status": "applied", "message": "受管文件已应用；Agent 加载状态尚未核验"}
        except (ValueError, OSError) as error:
            blocked.update(owners)
            for rid in owners:
                report.results[rid] = {"status": "failed", "message": str(error)}
    state.set("resource_results", report.results)
    return report
