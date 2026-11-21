---
name: easygo-deploy-dev-hot-update
description: Hot-update EasyGo packages with script/deploy-dev.py onto a live robot container, with optional SSH hop-chain routing. Use when the user asks to 热更新/deploy using deploy-dev.py and expects package/runtime verification.
---

# EasyGo Deploy-Dev Hot Update

Use the actual EasyGo workspace requested by the user. Deploy only when the user authorizes a hot update/deployment; build/package-only requests authorize no target writes.

Prepare and verify the package before the maintenance pause. If the user explicitly requests an immediate pause/stop, perform that authorized action first.

## Resolve Inputs

- Workspace containing `script/deploy-dev.py`, requested repos/packages, and source ref for each source repository.
- Target SSH alias or hostname/IP, optional ordered hop chain, authentication/port, architecture, and container.

Deploy the exact requested branch, tag, MR head/merge result, or SHA. Use the relevant source repo's `origin/main` when the user requests latest/main; ask if the source version is ambiguous. The EasyGo root ref is not the nested app ref.

Resolve destination tokens using the current SSH configuration (`ssh -G` when useful), without asking the user whether a token is an alias or IP. Bind each target to its resolved endpoint and verified robot identity for this run; re-resolve aliases on a new run. Use the configured username and prefer key-based login. When password login is required, obtain the credential from the approved local source and pass `--passwd` to the deployment script. Keep credentials out of the skill and evidence. Verify architecture/container from the target; `arm64` / `easygo` are common real-robot defaults.

For targets behind intermediate hosts, read [SSH routes](references/ssh-routes.md). Keep the same route, authentication, and port for preflight, deployment, and verification.

## Reuse the Deployment Script

Read `python3 script/deploy-dev.py --help` and the relevant definitions in [deploy-dev.py](../../../script/deploy-dev.py) when resolving behavior:

| Decision | Source of truth |
| --- | --- |
| Actual source checkout and package configuration | `repo_source_dir`, `REPOS` |
| Build recipe and included executables | `build_rust_cmd`, `REPOS`, artifact contents |
| Required deployment dependencies and their order | `expand_repo_aliases`, `expand_repos_for_implicit_deploy_deps` |
| Artifact identity and installation gates | `deploy_deb`, `inspect_deb_artifact`, `_deb_container_preflight_cmd`, `_dpkg_install_cmd` |
| Affected supervised programs | `PACKAGE_PROGRAMS` |

Select a recipe compatible with the requested source/release line. Confirm that it builds and packages every changed executable, including independent bridge programs; a mainline recipe may omit a maintenance-line executable.

Let the script compute dependency expansion, then compare its candidate versions with the target's installed SDK. For an app-only update, preserve already-compatible interface packages instead of installing older local interfaces merely because they were expanded. Keep SDK/resource overrides in isolated build prefixes. Select matching SDK inputs and package dependencies, and check that the app deb does not overwrite libraries owned by the retained interface packages.

Reuse the script's package metadata, transfer checksum, architecture, simulated-install, ABI and post-install checks. When the CLI cannot express the required source/SDK inputs or exact-artifact installation, inspect the current function signatures before using a small task-local adapter around those existing functions. Record the adapter and changed inputs; keep transfer/install gates in the deployment script rather than implementing another installer.

For an App build, resolve the HMI music manifest selected by the actual build before compilation. Check its provenance/ref, readability in the build container, and agreement with the source's HMI mappings. Select a matching manifest through a supported input or recorded input adapter; newer resources are not necessarily compatible. If the input cannot be resolved, report the precise gap before retrying. Preserve the HMI mappings and validation.

## Existing artifact requests

When the user requires installing an existing package without compilation, identify the artifact and the script's exact-artifact input first. Do not switch source checkouts, build, or repackage as a substitute. `--deploy-only` can repackage ordinary deb targets; it is not a selector for the previously inspected deb. If no supported path preserves the requested artifact, report that limitation before target writes. Preserve artifact checksum, architecture, dependency, installation and runtime checks.

## Workflow

1. **Resolve the source.** Fetch the relevant repos, resolve requested refs to SHAs, and inspect status. The script derives its workspace from its own location: pin the actual checkouts it will build, not just a separate App checkout.
2. **Protect local work.** If pinning would disturb unrelated WIP, use an isolated EasyGo workspace with the required nested source checkouts. Verify the script resolves those isolated paths. When an in-place switch is authorized and allowed by local rules, record its original branch/HEAD and exact temporary stash, and restore them on success and failure. Keep the selected build source fixed through installation.
3. **Preflight without pausing.** Read each target's identity, container, architecture, package versions/audit and relevant service/localization state. Stop on identity mismatch or an unhealthy package state. Routine maintenance stays read-only here; an explicit immediate-stop request is handled first.
4. **Prepare the artifact.** Allocate a unique evidence directory for each attempt. Record the root SHA, actual script path/hash, source SHAs/status, SDK/resource inputs, target/container and command shape without credentials. Use the current build/package-only mode, then verify the artifact identity, included executables, architecture, dependencies and checksum. Perform available non-disruptive installation preflight checks and retain a matching rollback artifact when available. Save full stdout/stderr and the original exit status; retries preserve earlier attempts. Build once for targets whose architecture and SDK inputs match.
5. **Pause and capture just before installation.** Process one target through installation and recovery before pausing the next, unless the user requested a fleet-wide stop. With the verified artifact ready, perform the authorized pause and confirm actual stopped motion across fresh samples. For updates that restart localization or require pose recovery, save a durable snapshot tied to robot identity, map, capture/source timestamps, units, pose and localization confidence/state. A pose read while the vehicle was moving during the build is not the installation snapshot. Missing or unlocalized data must not overwrite the last valid snapshot with zeros.
6. **Install the prepared artifact.** Use the script's supported exact-artifact path/functions and their gates. If the chosen entry point repackages, inspect and verify the resulting artifact again before installing; do not assume it is the earlier file. Stop/start only the programs selected by the script. For a dependency update followed by installation failure, read [partial-install recovery](references/partial-install-recovery.md).
7. **Verify runtime independently.** Require installed versions matching the artifact and an empty `dpkg --audit`; check the relevant supervised programs/processes and fresh startup logs/APIs. For changed executables, compare the actual running image (for example `/proc/<pid>/exe`) with the artifact hash, not only the installed file or process name. Verify retained SDK packages stayed at their intended versions.
8. **Recover localization when required.** Wait for initialization readiness. Restore only for the same identified robot/map and unchanged physical position while stationary; use the current API's units and arguments. Require fresh localized state and compare the accepted pose with the saved request, reporting the position/heading difference. An accepted request alone is not recovery. If the robot moved or its location is uncertain, do not reuse the old pose. If manual e-stop or missing scan/odometry blocks recovery, preserve the stop state and report the observed blocker and partial result; package installation is not localization success. Releasing e-stop, resetting CAN or starting a test navigation task needs its own authorization.
9. **Restore local state.** Restore any original checkouts/WIP switched by this workflow on success and failure. Confirm status before removing a restored temporary stash. Remove only disposable helpers; retain artifacts, input adapters and evidence needed for review or rollback.

## Remote Check Commands

For checks spanning the local shell, SSH and a container shell, quote each layer once. Prefer structured arguments with `subprocess.run` and `shlex.join`, or a quoted here-document, over repeated nested escaping. Preserve format strings such as `dpkg-query -f='${binary:Package}\t${Version}\t${db:Status-Status}\n'` literally until they reach `dpkg-query`.

When feeding a script through SSH to a container's `bash -s`, forward stdin with `docker exec -i`. Quoting does not make Docker forward stdin. Confirm every required probe ran, preserve its exit status and require the expected fields. Distinguish a confirmed empty `dpkg --audit` from a script that never ran.

Remote identity/time checks must expand on the remote host. Preserve the original command exit status when filtering output.

## Report

Report each target separately: exact source SHA, artifact/version/checksum, root/script revision, changed build inputs, evidence directory, command/exit status, package audit, running-image verification, initialization and localization result. Include the saved-pose record and recovery difference when applicable, and confirm local-work restoration. Keep CI, package build, runtime, localization and motion/avoidance acceptance distinct; identify failed, blocked or unverified stages explicitly.
