import json
import re
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import quote, urlparse

from .repository import git


def project_location(repo):
    remote = git(repo, "remote", "get-url", "origin")
    if remote.startswith(("http://", "https://", "ssh://")):
        parsed = urlparse(remote)
        host, project = parsed.hostname, parsed.path.lstrip("/")
    else:
        match = re.fullmatch(r"(?:[^@]+@)?([^:/]+):(.+)", remote)
        if not match:
            raise ValueError("MR submission requires a GitLab HTTP or SSH origin")
        host, project = match.groups()
    return host, project.removesuffix(".git")


def api(host, endpoint, body=None, method="POST"):
    argv = ["glab", "api", endpoint, "--hostname", host]
    with tempfile.TemporaryDirectory(prefix="sm-mr-") as temporary:
        if body is not None:
            path = Path(temporary) / "body.json"
            path.write_text(json.dumps(body, ensure_ascii=False))
            argv += ["--method", method, "--input", str(path), "--header", "Content-Type: application/json"]
        result = subprocess.run(argv, check=True, capture_output=True, text=True, timeout=120)
    return json.loads(result.stdout)


def submit_mr(proposal):
    host, project = project_location(proposal.directory)
    endpoint = f"projects/{quote(project, safe='')}/merge_requests"
    existing = api(host, endpoint + "?state=opened&source_branch=" + quote(proposal.branch, safe=""))
    git(proposal.directory, "push", "origin", proposal.branch)
    title = proposal.change.reason.splitlines()[0][:120]
    if proposal.change.validation.get("status") != "passed":
        title = "Draft: " + title
    body = {"title": title, "description": proposal.description()}
    if existing:
        mr = existing[0]
        api(host, f"{endpoint}/{mr['iid']}", body, method="PUT")
    else:
        mr = api(host, endpoint, {**body, "source_branch": proposal.branch, "target_branch": "main", "remove_source_branch": True})
    confirmed = api(host, f"{endpoint}/{mr['iid']}")
    if confirmed.get("source_branch") != proposal.branch or confirmed.get("target_branch") != "main":
        raise ValueError("GitLab MR result does not match the proposed branches")
    proposal.mr_url = confirmed["web_url"]
    proposal.status = "submitted"
    proposal.save()
    return proposal.mr_url
