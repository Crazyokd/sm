import json
import subprocess

import pytest


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], check=True,
                          capture_output=True, text=True).stdout.strip()


@pytest.fixture
def repo_pair(tmp_path):
    remote = tmp_path / "remote.git"
    work = tmp_path / "work"
    subprocess.run(["git", "init", "--bare", str(remote)], check=True, capture_output=True)
    subprocess.run(["git", "init", "-b", "main", str(work)], check=True, capture_output=True)
    git(work, "config", "user.name", "SM Test")
    git(work, "config", "user.email", "sm@example.invalid")
    git(work, "remote", "add", "origin", str(remote))
    (work / "catalog.json").write_text(json.dumps({"projects": {}, "resources": []}))
    git(work, "add", ".")
    git(work, "commit", "-m", "Initial catalog")
    git(work, "push", "origin", "main")
    git(remote, "symbolic-ref", "HEAD", "refs/heads/main")
    return work, remote


def skill_catalog(scope="user", project=None):
    return {"projects": {"app": "~/repo/app"} if project else {}, "resources": [
        {"id": "review", "kind": "skill", "origin": "self", "scope": scope,
         "project": project, "payload": "resources/review"}]}


def publish_skill(work, text="Approved", scope="user", project=None):
    folder = work / "resources/review"
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "SKILL.md").write_text(text)
    (work / "catalog.json").write_text(json.dumps(skill_catalog(scope, project)))
    git(work, "add", ".")
    git(work, "commit", "-m", text)
    git(work, "push", "origin", "main")


@pytest.fixture
def two_machine_case(repo_pair, tmp_path, monkeypatch):
    import shutil
    from pathlib import Path
    from types import SimpleNamespace
    from sm.state import State
    import sm.service as service
    import sm.gitlab as gitlab
    work, remote = repo_pair
    source = Path(__file__).parent / "fixtures/acceptance"
    publish_skill(work, (source / "SKILL.md").read_text(), "project", "app")
    shutil.copy(source / "evals.json", work / "resources/review/evals.json")
    git(work, "add", ".")
    git(work, "commit", "-m", "Approved evaluation cases")
    git(work, "push", "origin", "main")
    homes = tuple(tmp_path / name for name in ("machine-a", "machine-b"))
    for home in homes:
        (home / "repo/app").mkdir(parents=True)
    monkeypatch.setattr(service, "systemctl", lambda *args: None)
    monkeypatch.setattr(gitlab, "project_location", lambda _: ("git.example.invalid", "me/sm"))
    requests, mrs = [], []
    def api(host, endpoint, body=None, method="POST"):
        requests.append((endpoint, body))
        if body is not None:
            if method == "PUT":
                mr = mrs[int(endpoint.rsplit("/", 1)[1]) - 1]
                mr.update(body)
                return mr
            mr = {**body, "iid": len(mrs) + 1, "web_url": f"https://git.example.invalid/mr/{len(mrs)+1}"}
            mrs.append(mr)
            return mr
        if "?" in endpoint:
            from urllib.parse import parse_qs, urlsplit
            branch = parse_qs(urlsplit(endpoint).query)["source_branch"][0]
            return [mr for mr in mrs if mr["source_branch"] == branch]
        return mrs[int(endpoint.rsplit("/", 1)[1]) - 1]
    monkeypatch.setattr(gitlab, "api", api)
    return SimpleNamespace(repo=work, remote=remote, homes=homes, requests=requests, mrs=mrs,
                           state=lambda home: State(home / ".local/state/sm/state.sqlite3"))
