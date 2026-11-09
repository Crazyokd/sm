---
name: easygo-package-tag-mr
description: Tag an EasyGo child repository package version, usually from latest origin/main, update the root docker/packages.env version pin when needed, validate it, and submit an EasyGo root MR. Use when the user asks to 打tag, 发包tag, 更新packages.env, or create an MR for a new EasyGo package version.
---

# EasyGo Package Tag MR

Use this for the EasyGo workspace root that contains `easygo.repos`, `script/`, `docker/packages.env`, and `src/*` child repositories.

This workflow mutates remote Git state when pushing a tag, and later mutates the EasyGo root repository when opening an MR. Do not merge the MR or deploy anything unless the user separately asks.

## Rules

- Treat the EasyGo root repository and every `src/*` Git repository as separate repositories.
- A short request like “为 easygo-app 打 tag” means: target `src/easygo-app` at latest `origin/main`, reuse an existing tag on that commit if present, then update `EASYGO_APP_VERSION` in `docker/packages.env` only if it differs.
- For latest `origin/main`, fetch and resolve the target child repository's remote ref without switching its checkout. Run `easygo-force-sync` only when the user explicitly requests destructive workspace synchronization; a tagging request alone does not authorize it.
- Preserve the entry root and child checkouts' branches and WIP. Create a separate EasyGo root Git worktree for the package-pin MR; do not switch, stash, or reset the shared entry checkout as part of this workflow.
- If the tag name/version is missing or ambiguous and the target ref has no existing package tag, ask before creating the tag. Do not invent a release version silently.
- If the target ref already has a package tag, reuse that tag and do not create or push a duplicate tag. If the user explicitly requested a different tag name for the same ref, stop and ask before creating a second tag.
- Prefer `docker/packages.env`; if the user says `docker/package.env`, verify the actual path instead of creating a new file.
- Update only the relevant `*_VERSION=` lines in `docker/packages.env`. Do not change package source code in the EasyGo root MR.
- For package tags, prefer annotated tags when the repo already uses tag objects. Match the target repo's existing tag style if it differs.
- Before changing `packages.env`, confirm the exact tag SHA and actual package publication; a tag or green tag creation alone is not publication evidence. Keep unavailable publication explicit and do not claim downstream readiness.
- Run `python3 script/release-real.py check-env` after editing `docker/packages.env`. Treat failures as blockers, not warnings.

## Resolve Inputs

Identify:

- `WORKSPACE`: EasyGo root entry checkout.
- `ROOT_WORKTREE`: absolute path to the separate EasyGo root Git worktree for this task's package-pin MR.
- `CHILD_REPO`: the child repository to tag, such as `src/easygo-app` or `src/Vda5050_ROS2`.
- `SOURCE_REF`: usually `origin/main`, or the exact SHA/branch/tag requested by the user.
- `TAG_NAME`: package tag, usually `vX.Y.Z`; may be discovered from an existing tag on `SOURCE_REF`.
- `PACKAGE_ENV_VAR`: the `docker/packages.env` variable to update, such as `EASYGO_APP_VERSION` or `VDA5050_VERSION`.
- `MR_TARGET_BRANCH`: usually `main` unless the user requests a release branch.

Derive `PACKAGE_ENV_VAR` from `docker/packages.env`, `script/ci/install-sim-apt-pins.sh`, and Dockerfile package pins. Do not guess when multiple variables map to the same child repo, such as shared follow packages.

Common defaults:

- `easygo-app` -> `CHILD_REPO=src/easygo-app`, `SOURCE_REF=origin/main`, `PACKAGE_ENV_VAR=EASYGO_APP_VERSION`
- `vda5050` -> `CHILD_REPO=src/Vda5050_ROS2`, `SOURCE_REF=origin/main`, `PACKAGE_ENV_VAR=VDA5050_VERSION`

## Workflow

Note: the snippets below deliberately avoid positional fields such as `$N` and the skill's `NAME` / `ARGUMENTS` placeholders, because tokens of that shape are substituted when this file is loaded. Keep new snippets in the same style; use `sed` or parameter expansion instead of positional awk fields.

1. Prepare the workspace.

```bash
cd "$WORKSPACE"
```

Record the entry root and target child checkout's branch, HEAD, and working-tree status. Fetch the target child repo without changing its checkout:

```bash
git -C "$CHILD_REPO" fetch origin --prune --tags
git -C "$CHILD_REPO" log -1 --oneline --decorate "$SOURCE_REF"
```

2. Verify the tag target and discover any existing tag on it.

```bash
TARGET_SHA="$(git -C "$CHILD_REPO" rev-parse "$SOURCE_REF")"
REQUESTED_TAG_NAME="${TAG_NAME:-}"
EXISTING_TAG="$(git -C "$CHILD_REPO" tag --points-at "$TARGET_SHA" --list 'v*' --sort=-v:refname | head -n 1)"

if [ -n "$EXISTING_TAG" ]; then
  if [ -n "$REQUESTED_TAG_NAME" ] && [ "$REQUESTED_TAG_NAME" != "$EXISTING_TAG" ]; then
    echo "target $TARGET_SHA already has tag $EXISTING_TAG; not creating duplicate tag $REQUESTED_TAG_NAME" >&2
    exit 1
  fi
  TAG_NAME="$EXISTING_TAG"
  if git -C "$CHILD_REPO" ls-remote --exit-code --tags origin "refs/tags/$TAG_NAME" >/dev/null; then
    TAG_ALREADY_PRESENT=1
  else
    TAG_ALREADY_PRESENT=0
  fi
else
  [ -n "${TAG_NAME:-}" ] || { echo "missing TAG_NAME and target has no existing package tag" >&2; exit 1; }
  if git -C "$CHILD_REPO" ls-remote --exit-code --tags origin "refs/tags/$TAG_NAME" >/dev/null; then
    echo "remote tag already exists: $TAG_NAME" >&2
    exit 1
  fi
  TAG_ALREADY_PRESENT=0
fi
```

If the requested remote tag exists elsewhere, stop and compare it with `TARGET_SHA`; never move an existing remote tag without explicit force-tag authorization.

3. Create and push the child-repo tag only when needed.

```bash
if [ "$TAG_ALREADY_PRESENT" = 1 ]; then
  git -C "$CHILD_REPO" show --no-patch --decorate "$TAG_NAME"
else
  if git -C "$CHILD_REPO" rev-parse -q --verify "refs/tags/$TAG_NAME" >/dev/null; then
    LOCAL_TAG_SHA="$(git -C "$CHILD_REPO" rev-list -n 1 "$TAG_NAME")"
    [ "$LOCAL_TAG_SHA" = "$TARGET_SHA" ] || { echo "local tag points at $LOCAL_TAG_SHA, expected $TARGET_SHA" >&2; exit 1; }
  fi

  LATEST_TAG="$(git -C "$CHILD_REPO" tag --list 'v*' --sort=-v:refname | head -n 1)"
  if [ -n "$LATEST_TAG" ] && [ "$(git -C "$CHILD_REPO" cat-file -t "refs/tags/$LATEST_TAG")" = tag ]; then
    git -C "$CHILD_REPO" tag -a "$TAG_NAME" "$TARGET_SHA" -m "$TAG_NAME"
  else
    git -C "$CHILD_REPO" tag "$TAG_NAME" "$TARGET_SHA"
  fi

  git -C "$CHILD_REPO" show --no-patch --decorate "$TAG_NAME"
  git -C "$CHILD_REPO" push origin "refs/tags/$TAG_NAME"
fi
```

4. Update the EasyGo root version pin in a separate worktree only if needed.

Prefer the current harness worktree tool when available; the shell below is the manual fallback.

```bash
case "$ROOT_WORKTREE" in /*) ;; *) echo "ROOT_WORKTREE must be absolute" >&2; exit 1;; esac
PIN_VERSION="${TAG_NAME#v}"
git fetch origin --prune
TARGET_PACKAGE_ENV="$(git show "origin/$MR_TARGET_BRANCH:docker/packages.env")" || exit 1
CURRENT_PIN="$(printf '%s\n' "$TARGET_PACKAGE_ENV" | sed -n "s/^${PACKAGE_ENV_VAR}=//p")"
[ -n "$CURRENT_PIN" ] || { echo "target branch package variable missing" >&2; exit 1; }
if [ "$CURRENT_PIN" = "$PIN_VERSION" ]; then
  echo "target branch already has $PACKAGE_ENV_VAR=$PIN_VERSION; no EasyGo root MR needed"
  exit 0
fi

git worktree add -b "chore/package-$TAG_NAME" "$ROOT_WORKTREE" "origin/$MR_TARGET_BRANCH" || exit 1
cg-worktree-index "$ROOT_WORKTREE" || exit 1
cd "$ROOT_WORKTREE" || exit 1
```

Choose an unused worktree path. If the branch or worktree already exists, inspect its ownership and state before reusing it; do not switch the shared checkout as a fallback. Initialize the worktree according to the project's current worktree/index instructions.

Run all subsequent file edits, generation, validation, commits, and MR branch operations from `ROOT_WORKTREE`.

Edit only the exact version line in `docker/packages.env`, for example:

```text
EASYGO_APP_VERSION=0.4.139
```

Version values in `docker/packages.env` usually omit the leading `v` from `TAG_NAME`.

5. Validate and commit.

```bash
cd "$ROOT_WORKTREE" || exit 1
if [ -f script/ci/gen-image-env-subsets.py ]; then
  python3 script/ci/gen-image-env-subsets.py || exit 1
  python3 script/ci/gen-image-env-subsets.py --check || exit 1
fi
python3 script/release-real.py check-env || exit 1
git diff -- docker/packages.env docker/real/layers docker/sim/layers
git status --short
git add -- docker/packages.env
if [ -f script/ci/gen-image-env-subsets.py ]; then
  # Inspect the diff, then list only the exact generated files changed by this pin.
  # Example for an App-only pin; do not stage whole layer directories.
  git add -- docker/real/layers/app.env docker/sim/layers/app.env
fi
git commit -m "chore: update package pin for $TAG_NAME"
```

6. From `ROOT_WORKTREE`, push the branch and submit an MR to the target branch. Use the available GitLab workflow or `glab`; report the MR URL and CI state, but do not claim CI passed until it actually has.

## Report

Include:

- child repo path, source ref, and target SHA
- tag name and whether it was reused or pushed
- `docker/packages.env` variable changed
- `release-real.py check-env` result
- EasyGo root worktree path, branch, commit SHA, MR URL, and current CI state
- final entry root and child checkout status compared with the recorded baseline; report unexpected changes without overwriting another task's work

When filtering generator, publication, or validation logs, preserve the original exit status with `pipefail` or a separately captured status. Report the MR current head SHA, its matching pipeline, and package publication separately. A different checkout pin cannot establish that the requested target branch is already current.
