#!/usr/bin/env bash
# 强制同步工作区根仓 + src/ 下所有一级独立 git 仓到 origin/main
# （无 origin/main 时退到远端默认分支）。未跟踪文件保留，不做 git clean。
set -u

reset_repo() {
  local dir="$1"
  if ! git -C "$dir" rev-parse --git-dir >/dev/null 2>&1; then
    return 0
  fi
  # 只处理一级边界：dir 本身是仓库根
  local top
  top=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null) || return 0
  if [ "$(realpath "$top")" != "$(realpath "$dir")" ]; then
    return 0
  fi

  printf '== %s\n' "$dir"
  if ! git -C "$dir" remote get-url origin >/dev/null 2>&1; then
    printf '   BLOCKED: 无 origin 远端\n'
    return 1
  fi
  if ! git -C "$dir" fetch origin --prune; then
    # 远端强推过标签/分支时普通 fetch 会被拒，按 force-sync 语义强制对齐
    if ! git -C "$dir" fetch origin --prune --force --tags; then
      printf '   BLOCKED: fetch 失败\n'
      return 1
    fi
  fi

  local target=""
  if git -C "$dir" rev-parse --verify --quiet origin/main >/dev/null; then
    target="origin/main"
  else
    local def
    def=$(LC_ALL=C git -C "$dir" remote show origin 2>/dev/null | sed -n 's/.*HEAD branch: //p')
    if [ -n "$def" ] && [ "$def" != "(unknown)" ] \
      && git -C "$dir" rev-parse --verify --quiet "origin/$def" >/dev/null; then
      target="origin/$def"
    fi
  fi
  if [ -z "$target" ]; then
    printf '   BLOCKED: 无 origin/main 且无可用远端默认分支\n'
    return 1
  fi

  # checkout -f can remove untracked paths even without git clean.
  local path parent
  local -A untracked=() untracked_parents=()
  while IFS= read -r -d '' path; do
    path=${path%/}
    untracked["$path"]=1
    parent=$path
    while [[ "$parent" == */* ]]; do
      parent=${parent%/*}
      untracked_parents["$parent"]=1
    done
  done < <(git -C "$dir" ls-files --others --directory -z)
  while IFS= read -r -d '' path; do
    if [[ ${untracked_parents["$path"]+present} ]]; then
      printf '   BLOCKED: 未跟踪目录将被目标文件覆盖: %s\n' "$path"
      return 1
    fi
    parent=$path
    while :; do
      if [[ ${untracked["$parent"]+present} ]]; then
        printf '   BLOCKED: 未跟踪路径与目标分支冲突: %s\n' "$parent"
        return 1
      fi
      [[ "$parent" == */* ]] || break
      parent=${parent%/*}
    done
  done < <(git -C "$dir" ls-tree -r --name-only -z "$target")

  git -C "$dir" checkout -q -f -B main "$target" \
    && git -C "$dir" reset --hard -q "$target" \
    && printf '   OK -> %s (%s)\n' "$target" "$(git -C "$dir" rev-parse --short HEAD)"
}

rc=0
reset_repo "." || rc=1
for d in src/*/; do
  reset_repo "${d%/}" || rc=1
done
exit "$rc"
