---
name: easygo-deploy-dev-hot-update
description: Hot-update EasyGo packages with script/deploy-dev.py onto a live robot container, with optional SSH hop-chain routing. Use when the user asks to 热更新/deploy using deploy-dev.py and expects package/runtime verification.
---

# EasyGo Deploy-Dev Hot Update

Use the actual EasyGo workspace requested by the user. Deploy only when the user authorizes a hot update/deployment; build/package-only requests authorize no target writes.

## Resolve Inputs

- Workspace containing `script/deploy-dev.py`, requested repos/packages, and source ref for each source repository.
- Target SSH alias or hostname/IP, optional ordered hop chain, authentication/port, architecture, and container.

Deploy the exact requested branch, tag, MR head/merge result, or SHA. Use the relevant source repo's `origin/main` when the user requests latest/main; ask if the source version is ambiguous. The EasyGo root ref is not the nested app ref.

Resolve destination tokens using the current SSH configuration (`ssh -G` when useful), without asking the user whether a token is an alias or IP. Use the username configured for the target. Prefer key-based login; when password login is required, obtain the credential from the approved local credential source and pass `--passwd` to the deployment script. Do not store passwords in this Skill or use a hard-coded fallback. Verify architecture/container from the target; `arm64` / `easygo` are common real-robot defaults.

For targets behind intermediate hosts, read [SSH routes](references/ssh-routes.md). Keep the same route, authentication, and port for preflight, deployment, and verification.

## Reuse the Deployment Script

Read `python3 script/deploy-dev.py --help` and the relevant definitions in [deploy-dev.py](../../../script/deploy-dev.py) when resolving behavior:

| Decision | Source of truth |
| --- | --- |
| Actual source checkout and package configuration | `repo_source_dir`, `REPOS` |
| Required deployment dependencies and their order | `expand_repo_aliases`, `expand_repos_for_implicit_deploy_deps` |
| Artifact identity and installation gates | `deploy_deb`, `inspect_deb_artifact`, `_deb_container_preflight_cmd`, `_dpkg_install_cmd` |
| Affected supervised programs | `PACKAGE_PROGRAMS` |

Pass the requested repo targets to the script and let it expand dependencies. Reuse its package metadata, transfer checksum, architecture, simulated-install, and post-install checks; do not maintain a second dependency list or reimplement these gates in the skill.

## Existing artifact requests

When the user requires installing an existing package without compilation, first identify the artifact and check whether the current script supports that exact input. Do not switch or stash source checkouts, build, or repackage as a substitute. `--deploy-only` can still package source; if there is no supported artifact input, report that limitation before any target write. Preserve artifact checksum, architecture, dependency, installation and runtime checks.

## Workflow

1. **Resolve the source.** Fetch the relevant source repos, resolve each requested ref to a SHA, and inspect status. The script derives its workspace from its own location and has no source-worktree/ref selector: pin the actual checkouts it will build.
2. **Protect local work.** Record original branch/HEAD and WIP for each checkout being switched. Stash tracked/untracked WIP under a unique `codex-deploy-*` name, then detach to the resolved SHA. Keep that source state through build and deployment.
3. **Preflight the target.** Confirm SSH access, host identity, container, architecture, current packages, package audit, and relevant service state. Stop if access fails or the container/package state is not healthy enough to deploy.
4. **Build and deploy through the script.** Before each launch, allocate a unique evidence directory for that attempt. Record the EasyGo root SHA, actual script path and content hash, source checkout SHAs/status, resolved target/container, and command shape without credentials. For a script extracted from another ref, also record its source path and resolved commit SHA. Save complete stdout/stderr, the original command exit status, and artifact metadata/checksums. Retries keep earlier attempts' logs and metadata intact, including when the script, source, or target changes.

   The normal invocation performs build, packaging, gates, and installation:

   ```bash
   cd "$WORKSPACE"
   python3 script/deploy-dev.py "$DEPLOY_HOST" "${REQUESTED_REPOS[@]}" --arch "$ARCH" --container "$CONTAINER"
   ```

   Add the resolved `--passwd` / `--port` when needed. For build/package-only requests, use the corresponding flag from `--help`. For an installation failure after a dependency update, read [partial-install recovery](references/partial-install-recovery.md).
5. **Verify independently.** On the target, compare installed package versions with the deployed artifact metadata and require an empty `dpkg --audit`. Check the relevant supervised programs/processes and bounded fresh logs or APIs for runtime startup. Treat stale versions, audit output, or missing expected runtime processes as failure. For app deployments, report initialization/localization separately from any observed navigation or charging test.
6. **Restore local state.** Return switched checkouts to their original branch/HEAD and restore their WIP. Confirm final status and remove only temporary stashes/wrappers created by this workflow.

## Report

Include exact source refs/SHAs, package versions, target/container, command shape, package-audit and runtime results, and whether local state was restored. For each deployment attempt, identify the root/script revision, evidence directory, exit status, and any changed inputs. State which functional/device checks were actually observed. Mention build fallbacks only if they happened.

Remote identity and time checks must expand on the remote host: quote remote commands so local shell substitutions cannot supply the reported hostname or timestamp. Preserve the original command exit status when filtering output.
