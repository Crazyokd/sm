#!/usr/bin/env bash
set -euo pipefail

target_user=""
target_pass=""

usage() {
  echo "usage: $0 [--target-user USER] [--target-pass PASS] <wrapper-host> <hop1> [<hop2> ...] -- <target-alias-or-ip> [output-dir]" >&2
  echo "       $0 [--target-user USER] [--target-pass PASS] <wrapper-host> <hop1> <target-alias-or-ip> [output-dir]" >&2
  exit 2
}

while (($#)); do
  case "$1" in
    --target-user)
      target_user="${2:?missing --target-user value}"
      shift 2
      ;;
    --target-pass)
      target_pass="${2:?missing --target-pass value}"
      shift 2
      ;;
    --)
      break
      ;;
    -*)
      echo "unknown option: $1" >&2
      exit 2
      ;;
    *)
      break
      ;;
  esac
done

(($# >= 3)) || usage

wrapper_host="$1"
shift

args=("$@")
sep=-1
for i in "${!args[@]}"; do
  if [[ "${args[$i]}" == "--" ]]; then
    sep="$i"
    break
  fi
done

if ((sep >= 0)); then
  ((sep > 0)) || usage
  hops=("${args[@]:0:sep}")
  rest=("${args[@]:sep+1}")
  ((${#rest[@]} == 1 || ${#rest[@]} == 2)) || usage
  target="${rest[0]}"
  out="${rest[1]:-}"
else
  (($# == 2 || $# == 3)) || usage
  hops=("$1")
  target="$2"
  out="${3:-}"
fi

out="${out:-$(mktemp -d /tmp/easygo-deploy-ssh.XXXXXX)}"

target_spec="$target"
if [[ -n "$target_user" && "$target_spec" != *@* ]]; then
  target_spec="$target_user@$target_spec"
fi

mkdir -p "$out/bin"

{
  printf '#!/usr/bin/env bash\n'
  printf 'set -euo pipefail\n\n'
  printf 'wrapper_host=%q\n' "$wrapper_host"
  printf 'target_spec=%q\n' "$target_spec"
  printf 'target_pass=%q\n' "$target_pass"
  printf 'hops=('
  printf '%q ' "${hops[@]}"
  printf ')\n\n'
  cat <<'EOF'
build_hop_cmd() {
  local cmd
  printf -v cmd '%q ' "$@"

  local i
  for ((i=${#hops[@]} - 1; i >= 1; i--)); do
    local wrapped=(/usr/bin/ssh "${ssh_opts[@]}" "${hops[$i]}" "$cmd")
    printf -v cmd '%q ' "${wrapped[@]}"
  done

  printf '%s' "$cmd"
}

ssh_opts=()
while (($#)); do
  case "$1" in
    -o|-p|-i|-F|-l|-S|-c|-m|-J)
      ssh_opts+=("$1" "$2")
      shift 2
      ;;
    --)
      shift
      break
      ;;
    -*)
      ssh_opts+=("$1")
      shift
      ;;
    *)
      break
      ;;
  esac
done

host="${1:-}"
if [[ -z "$host" ]]; then
  exec /usr/bin/ssh "${ssh_opts[@]}"
fi
shift

if [[ "$host" == "$wrapper_host" ]]; then
  final=(/usr/bin/ssh "${ssh_opts[@]}" "$target_spec" "$@")
  if [[ -n "$target_pass" ]]; then
    final=(sshpass -p "$target_pass" "${final[@]}")
  fi
  remote_cmd="$(build_hop_cmd "${final[@]}")"
  exec /usr/bin/ssh "${ssh_opts[@]}" "${hops[0]}" "$remote_cmd"
fi

exec /usr/bin/ssh "${ssh_opts[@]}" "$host" "$@"
EOF
} > "$out/bin/ssh"

{
  printf '#!/usr/bin/env bash\n'
  printf 'set -euo pipefail\n\n'
  printf 'wrapper_host=%q\n' "$wrapper_host"
  printf 'target_spec=%q\n' "$target_spec"
  printf 'target_pass=%q\n' "$target_pass"
  printf 'hops=('
  printf '%q ' "${hops[@]}"
  printf ')\n\n'
  cat <<'EOF'
build_hop_cmd() {
  local cmd
  printf -v cmd '%q ' "$@"

  local i
  for ((i=${#hops[@]} - 1; i >= 1; i--)); do
    local wrapped=(/usr/bin/ssh "${ssh_opts[@]}" "${hops[$i]}" "$cmd")
    printf -v cmd '%q ' "${wrapped[@]}"
  done

  printf '%s' "$cmd"
}

scp_opts=()
ssh_opts=()
recursive=0
while (($#)); do
  case "$1" in
    -o|-i|-F|-l|-S|-c|-m|-J)
      scp_opts+=("$1" "$2")
      ssh_opts+=("$1" "$2")
      shift 2
      ;;
    -P)
      scp_opts+=("$1" "$2")
      ssh_opts+=("-p" "$2")
      shift 2
      ;;
    -r)
      recursive=1
      scp_opts+=("$1")
      shift
      ;;
    -q|-v|-C|-4|-6)
      scp_opts+=("$1")
      ssh_opts+=("$1")
      shift
      ;;
    -p)
      scp_opts+=("$1")
      shift
      ;;
    --)
      shift
      break
      ;;
    -*)
      scp_opts+=("$1")
      shift
      ;;
    *)
      break
      ;;
  esac
done

if (($# != 2)); then
  exec /usr/bin/scp "${scp_opts[@]}" "$@"
fi

src="$1"
dst="$2"
prefix="${wrapper_host}:"
if [[ "$dst" == "$prefix"* && "$recursive" -eq 0 && -f "$src" ]]; then
  remote_path="${dst#"$prefix"}"
  [[ -n "$remote_path" ]] || exec /usr/bin/scp "${scp_opts[@]}" "$src" "$dst"
  printf -v quoted_remote_path '%q' "$remote_path"
  final=(/usr/bin/ssh "${ssh_opts[@]}" "$target_spec" "cat > $quoted_remote_path")
  if [[ -n "$target_pass" ]]; then
    final=(sshpass -p "$target_pass" "${final[@]}")
  fi
  remote_cmd="$(build_hop_cmd "${final[@]}")"
  exec /usr/bin/ssh "${ssh_opts[@]}" "${hops[0]}" "$remote_cmd" < "$src"
fi

exec /usr/bin/scp "${scp_opts[@]}" "$src" "$dst"
EOF
} > "$out/bin/scp"

chmod +x "$out/bin/ssh" "$out/bin/scp"
printf '%s\n' "$out"
