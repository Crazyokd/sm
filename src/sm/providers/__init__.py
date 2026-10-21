import shlex
from pathlib import Path

from ..catalog import OwnedWrite, Resource


def payload_files(payload: Path):
    for path in sorted(payload.rglob("*")):
        if ".git" in path.relative_to(payload).parts:
            continue
        if path.is_symlink():
            raise ValueError(f"payload symlink is not accepted: {path.name}")
        if path.is_file():
            yield path, path.relative_to(payload)


def copy_payload(resource: Resource, payload: Path, target: Path):
    return [OwnedWrite(resource.id, target / rel, "file", path.read_bytes(), [], bool(path.stat().st_mode & 0o111))
            for path, rel in payload_files(payload)]


def hook_command(command: str, directory: Path) -> str:
    return shlex.join(["env", f"SM_RESOURCE_DIR={directory}", "sh", "-c", command])
