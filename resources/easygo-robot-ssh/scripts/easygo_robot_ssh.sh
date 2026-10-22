#!/usr/bin/env bash
# 功能：为 EasyGo 机器人/ARM 设备配置 SSH Host、安装本机公钥并验证免密登录。
# 具体做什么：从 skill 的 references/robots.tsv 读取目标机器清单，按 alias 生成或更新 ~/.ssh/config，使用 sshpass 把当前用户公钥写入远端 authorized_keys，并提供连接测试。
# 使用方法：bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh list|config|install-key|setup|test <alias>，或使用 --alias/--host/--user/--port 指定临时目标。
# 运行位置：在感知仓库根目录下执行；不要在 ARM 机器人上运行本脚本。脚本路径和清单路径都基于本 skill 目录自动计算。
set -euo pipefail

script_name="${0##*/}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skill_dir="$(cd "${script_dir}/.." && pwd)"
inventory_file="${EASYGO_ROBOT_INVENTORY:-${skill_dir}/references/robots.tsv}"
ssh_config_file="${EASYGO_SSH_CONFIG:-${HOME}/.ssh/config}"
identity_file="${EASYGO_SSH_IDENTITY:-}"
pubkey_path="${EASYGO_SSH_PUBKEY:-}"
password="${EASYGO_SSH_PASSWORD:-}"
custom_alias=""
custom_host=""
custom_user=""
custom_port="22"
custom_note="custom target"

usage() {
  cat <<USAGE
Usage:
  ${script_name} list
  ${script_name} config <alias>
  ${script_name} install-key <alias>
  ${script_name} setup <alias>
  ${script_name} test <alias>
  ${script_name} setup --alias <alias> --host <ip> --user <user> [--port <port>]

Options for custom targets:
  --alias ALIAS       SSH Host alias to create.
  --host HOST         Robot IP or hostname.
  --user USER         SSH user.
  --port PORT         SSH port. Default: 22.
  --note TEXT         Optional note printed in logs.

Environment:
  EASYGO_SSH_PASSWORD    Remote password for this command only. If unset, prompt interactively.
  EASYGO_SSH_IDENTITY    Private key path for SSH config. Default: auto-detect ~/.ssh/id_*.
  EASYGO_SSH_PUBKEY      Public key path. Default: private key + .pub or auto-detect.
  EASYGO_ROBOT_INVENTORY Robot TSV inventory. Default: skill references/robots.tsv.
  EASYGO_SSH_CONFIG      SSH config path. Default: ~/.ssh/config.
USAGE
}

log() { printf '[INFO] %s\n' "$*"; }
warn() { printf '[WARN] %s\n' "$*" >&2; }
fatal() { printf '[ERROR] %s\n' "$*" >&2; exit 1; }

require_inventory() {
  [[ -f "${inventory_file}" ]] || fatal "Inventory not found: ${inventory_file}"
}

list_targets() {
  require_inventory
  awk -F '\t' 'NR == 1 { next } NF >= 4 { printf "%-16s %s@%s:%s  %s\n", $1, $3, $2, $4, $5 }' "${inventory_file}"
}

load_target() {
  local alias="$1"
  require_inventory
  local line
  line="$(awk -F '\t' -v alias="${alias}" 'NR > 1 && $1 == alias { print; found=1; exit } END { if (!found) exit 1 }' "${inventory_file}")" || {
    warn "Unknown target alias: ${alias}"
    warn "Known targets:"
    list_targets >&2
    exit 2
  }
  IFS=$'\t' read -r target_alias target_host target_user target_port target_note <<< "${line}"
  [[ -n "${target_host}" && -n "${target_user}" && -n "${target_port}" ]] || fatal "Invalid inventory row for ${alias}"
}

parse_target_args() {
  local alias_arg=""

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --alias)
        [[ $# -ge 2 ]] || fatal "Missing value for --alias"
        custom_alias="$2"
        shift 2
        ;;
      --host)
        [[ $# -ge 2 ]] || fatal "Missing value for --host"
        custom_host="$2"
        shift 2
        ;;
      --user)
        [[ $# -ge 2 ]] || fatal "Missing value for --user"
        custom_user="$2"
        shift 2
        ;;
      --port)
        [[ $# -ge 2 ]] || fatal "Missing value for --port"
        custom_port="$2"
        shift 2
        ;;
      --note)
        [[ $# -ge 2 ]] || fatal "Missing value for --note"
        custom_note="$2"
        shift 2
        ;;
      --)
        shift
        break
        ;;
      -*)
        fatal "Unknown option: $1"
        ;;
      *)
        if [[ -n "${alias_arg}" ]]; then
          fatal "Only one alias argument is allowed"
        fi
        alias_arg="$1"
        shift
        ;;
    esac
  done

  if [[ -n "${custom_alias}${custom_host}${custom_user}" ]]; then
    [[ -n "${custom_alias}" && -n "${custom_host}" && -n "${custom_user}" ]] || \
      fatal "Custom target requires --alias, --host, and --user"
    target_alias="${custom_alias}"
    target_host="${custom_host}"
    target_user="${custom_user}"
    target_port="${custom_port}"
    target_note="${custom_note}"
    return 0
  fi

  [[ -n "${alias_arg}" ]] || { usage >&2; exit 2; }
  load_target "${alias_arg}"
}

ensure_port() {
  [[ "${target_port}" =~ ^[0-9]+$ ]] || fatal "Invalid SSH port for ${target_alias}: ${target_port}"
}

expand_path() {
  case "$1" in
    "~") printf '%s\n' "${HOME}" ;;
    "~/"*) printf '%s/%s\n' "${HOME}" "${1#~/}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

detect_identity() {
  local candidates=(
    "${HOME}/.ssh/id_ed25519"
    "${HOME}/.ssh/id_rsa"
    "${HOME}/.ssh/id_ecdsa"
  )
  local candidate
  for candidate in "${candidates[@]}"; do
    [[ -f "${candidate}" ]] && { printf '%s\n' "${candidate}"; return 0; }
  done
  return 1
}

ensure_identity() {
  if [[ -n "${identity_file}" ]]; then
    identity_file="$(expand_path "${identity_file}")"
  else
    identity_file="$(detect_identity)" || fatal "No SSH private key found. Create one on the current dev machine: ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519"
  fi
  [[ -f "${identity_file}" ]] || fatal "Private key not found: ${identity_file}"
  [[ "${identity_file}" != *.pub ]] || fatal "Identity file must be private key, not .pub: ${identity_file}"
}

ensure_pubkey() {
  ensure_identity
  if [[ -n "${pubkey_path}" ]]; then
    pubkey_path="$(expand_path "${pubkey_path}")"
  else
    pubkey_path="${identity_file}.pub"
  fi
  [[ -f "${pubkey_path}" ]] || fatal "Public key not found: ${pubkey_path}"
}

identity_for_config() {
  local home_prefix="${HOME}/"
  case "${identity_file}" in
    "${home_prefix}"*) printf '~/%s\n' "${identity_file#${home_prefix}}" ;;
    *) printf '%s\n' "${identity_file}" ;;
  esac
}

write_config() {
  ensure_identity
  ensure_port

  local config_dir tmp_file block_file config_identity
  config_dir="$(dirname "${ssh_config_file}")"
  mkdir -p "${config_dir}"
  [[ "${config_dir}" == "${HOME}/.ssh" ]] && chmod 700 "${config_dir}"
  touch "${ssh_config_file}"
  chmod 600 "${ssh_config_file}"

  config_identity="$(identity_for_config)"
  block_file="$(mktemp "${TMPDIR:-/tmp}/easygo-ssh-block.XXXXXX")"
  cat > "${block_file}" <<BLOCK
Host ${target_alias}
  HostName ${target_host}
  User ${target_user}
  Port ${target_port}
  IdentityFile ${config_identity}
  IdentitiesOnly yes
  AddKeysToAgent yes
  ServerAliveInterval 30
  ServerAliveCountMax 3
BLOCK

  tmp_file="$(mktemp "${ssh_config_file}.tmp.XXXXXX")"
  awk -v alias="${target_alias}" -v block_file="${block_file}" '
function read_block(   line) {
  while ((getline line < block_file) > 0) print line
  close(block_file)
}
function is_host_line(line) { return line ~ /^[[:space:]]*[Hh][Oo][Ss][Tt][[:space:]]+/ }
function host_matches(line,   n, fields, i) {
  n = split(line, fields, /[[:space:]]+/)
  for (i = 2; i <= n; i++) if (fields[i] == alias) return 1
  return 0
}
BEGIN { skipping = 0; replaced = 0 }
is_host_line($0) {
  if (skipping) skipping = 0
  if (host_matches($0)) {
    if (!replaced) { read_block(); replaced = 1 }
    skipping = 1
    next
  }
}
{ if (!skipping) print }
END {
  if (!replaced) {
    if (NR > 0) print ""
    read_block()
  }
}
' "${ssh_config_file}" > "${tmp_file}"
  mv "${tmp_file}" "${ssh_config_file}"
  rm -f "${block_file}"
  chmod 600 "${ssh_config_file}"
  log "SSH config updated: ${ssh_config_file}"
  log "Alias ${target_alias}: ${target_user}@${target_host}:${target_port}"
}

ensure_password() {
  if [[ -n "${password}" ]]; then
    return 0
  fi
  read -r -s -p "Password for ${target_user}@${target_host}:${target_port}: " password
  printf '\n'
  [[ -n "${password}" ]] || fatal "Password is empty"
}

install_key() {
  ensure_pubkey
  ensure_password
  command -v sshpass >/dev/null 2>&1 || fatal "sshpass not found on the current dev machine. Install it or provide another key-copy method."

  local pubkey
  pubkey="$(awk 'NF {print; exit}' "${pubkey_path}" | tr -d '\r')"
  [[ -n "${pubkey}" ]] || fatal "Public key is empty: ${pubkey_path}"

  log "Installing ${pubkey_path} to ${target_alias} (${target_user}@${target_host}:${target_port})"
  SSHPASS="${password}" sshpass -e ssh \
    -o StrictHostKeyChecking=accept-new \
    -p "${target_port}" "${target_user}@${target_host}" 'set -eu
      umask 077
      mkdir -p "$HOME/.ssh"
      touch "$HOME/.ssh/authorized_keys"
      chmod 700 "$HOME/.ssh"
      chmod 600 "$HOME/.ssh/authorized_keys"
      key=$(cat)
      if grep -qxF "$key" "$HOME/.ssh/authorized_keys"; then
        echo "Key already exists"
      else
        printf "%s\n" "$key" >> "$HOME/.ssh/authorized_keys"
        echo "Key added"
      fi
    ' <<< "${pubkey}"
}

test_login() {
  ssh -o BatchMode=yes "${target_alias}" 'hostname; uname -m; uname -s'
}

main() {
  local command="${1:-}"
  shift || true

  case "${command}" in
    list)
      list_targets
      ;;
    config|install-key|setup|test)
      parse_target_args "$@"
      case "${command}" in
        config) write_config ;;
        install-key) install_key ;;
        setup) write_config; install_key; test_login ;;
        test) test_login ;;
      esac
      ;;
    -h|--help|help|"")
      usage
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
