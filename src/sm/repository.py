import io
import subprocess
import tarfile
from pathlib import Path

from .catalog import Resource, contained


def git(repo: Path, *args) -> str:
    return subprocess.run(["git", "-C", str(repo), *args], check=True,
                          capture_output=True, text=True, timeout=120).stdout.strip()


def fetch_approved(repo: Path, output: Path) -> Path:
    git(repo, "fetch", "origin", "main")
    archive = subprocess.run(["git", "-C", str(repo), "archive", "origin/main"],
                             capture_output=True, check=True, timeout=120).stdout
    output.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        for member in tar.getmembers():
            if member.name != "catalog.json" and not member.name.startswith("resources/"):
                continue
            target = contained(output, member.name)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                with tar.extractfile(member) as source:
                    target.write_bytes(source.read())
                target.chmod(0o700 if member.mode & 0o111 else 0o600)
            else:
                raise ValueError(f"source link or special file is not accepted: {member.name}")
    return output


def fetch_upstream(resource: Resource, output: Path) -> Path:
    source = resource.upstream
    if source.get("git"):
        subprocess.run(["git", "clone", "--depth", "1", "--", source["git"], str(output)],
                       check=True, capture_output=True, timeout=120)
    else:
        argv = source["argv"]
        if not isinstance(argv, list) or not argv or not all(isinstance(a, str) for a in argv):
            raise ValueError("upstream argv must be a nonempty string array")
        output.mkdir(parents=True, exist_ok=True)
        subprocess.run([*argv, str(output.resolve())], cwd=resource.root,
                       check=True, capture_output=True, timeout=120)
    return output
