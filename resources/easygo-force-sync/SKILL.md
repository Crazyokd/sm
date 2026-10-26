---
name: easygo-force-sync
description: Force-sync the EasyGo workspace root repository plus every independent first-level Git repository under src/ to the latest origin/main, falling back to the remote default branch when origin/main is absent. Use when the user asks to forcibly update EasyGo, src packages, function packages, all packages, nested repos, or local conflicts should be resolved by remote code.
---

# EasyGo Force Sync

## Scope

Use this only from the EasyGo workspace root, the directory containing `easygo.repos`, `easygo_sim.repos`, `script/`, and `src/`.

The sync target is:

- the EasyGo root repository, `.`
- every first-level independent Git repository at `src/*`

Do not reset directories under `src/` that are not their own Git repository root. Do not recurse into nested submodules unless the user explicitly asks.

## Workflow

1. State that this is destructive for tracked files: local commits, branch position, and tracked edits in the listed repositories will be replaced by `origin/main`, or by the remote default branch when `origin/main` is absent.
2. From the EasyGo workspace root, run the script bundled with this Skill:

```bash
bash .agents/skills/easygo-force-sync/scripts/force-sync-origin-main.sh
```

3. If a repository has no `origin/main`, the script uses the remote HEAD/default branch reported by Git. If neither target exists, stop.
4. After success, verify the final state:

```bash
git status --short --branch
for d in . src/*; do
  [ -d "$d/.git" ] || continue
  printf '%s ' "$d"
  git -C "$d" status --short --branch | sed -n '1p'
done
```

5. Report:

- repos reset successfully
- repos skipped or blocked because neither `origin/main` nor a usable remote default branch exists
- remaining untracked files, if any

## Guardrails

- This skill intentionally keeps untracked files and directories.
- Never run `git clean` unless the user explicitly asks to delete untracked files too.
- Block a repository when an untracked or ignored path overlaps a tracked path in the target branch; keep its local contents intact.
- Prefer `origin/main`; use the remote default branch only when `origin/main` is absent.
- Treat `src/easygo-app`, `src/easygo-music-resources`, and other `src/*` entries as independent repository boundaries.
- SM installs this Skill and maintains its local Git exclusion so `git stash --include-untracked` leaves it in place. `git stash --all` also includes ignored files and can still remove it; restore the Skill through SM if that happens.
