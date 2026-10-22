#!/usr/bin/env bash
# 功能：静态体检感知 Flow 插件目录。
# 具体做什么：检查目录命名、stage 合法性、default.yaml type、导出宏、输入引用、个人路径和模型路径配置等常见问题；ROS2 adapter Flow 会同时检查 stage 级共享文件。
# 使用方法：bash .agents/skills/perception-flow-plugin/scripts/inspect_flow_plugin.sh pipeline/src/flow_impls/3.frontend/demo_frontend_flow
# 运行位置：在感知仓库根目录执行；脚本只做静态检查，不会修改文件。
set -euo pipefail

flow_dir="${1:-}"
failures=0
warnings=0

info() { printf '[INFO] %s\n' "$*"; }
ok() { printf '[OK]   %s\n' "$*"; }
warn() { printf '[WARN] %s\n' "$*"; warnings=$((warnings + 1)); }
fail() { printf '[FAIL] %s\n' "$*"; failures=$((failures + 1)); }

usage() {
  cat <<USAGE
Usage:
  ${0##*/} <flow_dir>

Example:
  bash .agents/skills/perception-flow-plugin/scripts/inspect_flow_plugin.sh \
    pipeline/src/flow_impls/3.frontend/demo_frontend_flow
USAGE
}

if [[ -z "${flow_dir}" || "${flow_dir}" == "-h" || "${flow_dir}" == "--help" ]]; then
  usage
  exit 2
fi

if [[ ! -d "${flow_dir}" ]]; then
  fail "Flow directory not found: ${flow_dir}"
  exit 1
fi

flow_dir="${flow_dir%/}"
flow_type="$(basename "${flow_dir}")"
stage_dir_abs="$(dirname "${flow_dir}")"
stage_dir="$(basename "${stage_dir_abs}")"
stage="${stage_dir#*.}"
scan_paths=("${flow_dir}")
if [[ "${flow_dir}" == scheduler/adapters/ros2/flow_plugins/* ]]; then
  while IFS= read -r shared_file; do
    scan_paths+=("${shared_file}")
  done < <(find "${stage_dir_abs}" -maxdepth 1 -type f \( -name '*.hpp' -o -name '*.h' -o -name '*.cpp' -o -name '*.cc' -o -name '*.cxx' \) | sort)
fi

case "${stage}" in
  cam|preprocess|frontend|backend|fusion|tracking|output)
    ok "stage: ${stage}"
    ;;
  *)
    fail "invalid stage '${stage}' from directory '${stage_dir}'"
    ;;
esac

if [[ "${stage_dir}" =~ ^[0-9]+\. ]]; then
  ok "stage directory has numeric prefix: ${stage_dir}"
else
  warn "stage directory does not use '<index>.<stage>' format: ${stage_dir}"
fi

if [[ -f "${flow_dir}/default.yaml" ]]; then
  ok "default.yaml exists"
  yaml_type="$(awk -F ':' '/^[[:space:]]*type[[:space:]]*:/ {gsub(/[ \t\"'"'"']/, "", $2); print $2; exit}' "${flow_dir}/default.yaml")"
  if [[ -z "${yaml_type}" ]]; then
    fail "default.yaml missing top-level type"
  elif [[ "${yaml_type}" == "${flow_type}" ]]; then
    ok "default.yaml type matches directory: ${yaml_type}"
  else
    fail "default.yaml type '${yaml_type}' does not match directory '${flow_type}'"
  fi
else
  fail "default.yaml missing"
fi

if [[ -f "${flow_dir}/README.md" ]]; then
  ok "README.md exists"
else
  warn "README.md missing; standard Flow template should document purpose, IO, params and validation"
fi

mapfile -t cpp_files < <(find "${flow_dir}" -maxdepth 1 -type f \( -name '*.cpp' -o -name '*.cc' -o -name '*.cxx' \) | sort)
mapfile -t hpp_files < <(find "${flow_dir}" -maxdepth 1 -type f \( -name '*.hpp' -o -name '*.h' \) | sort)

if (( ${#cpp_files[@]} == 0 )); then
  fail "no C++ source file found"
else
  ok "C++ source files: ${#cpp_files[@]}"
fi

if (( ${#hpp_files[@]} == 0 )); then
  warn "no header file found"
else
  ok "header files: ${#hpp_files[@]}"
fi

if grep -R "EXPORT_STAGE_FLOW_PLUGIN" "${flow_dir}" >/dev/null 2>&1; then
  ok "uses EXPORT_STAGE_FLOW_PLUGIN"
elif grep -R "EXPORT_FLOW_PLUGIN" "${flow_dir}" >/dev/null 2>&1; then
  warn "uses EXPORT_FLOW_PLUGIN; verify stage string manually"
else
  fail "missing flow export macro"
fi

if grep -R "GetInputRequirements" "${scan_paths[@]}" >/dev/null 2>&1; then
  ok "GetInputRequirements found"
else
  fail "GetInputRequirements missing"
fi

if grep -R "GetOutputRequirements" "${scan_paths[@]}" >/dev/null 2>&1; then
  ok "GetOutputRequirements found"
else
  fail "GetOutputRequirements missing"
fi

if grep -R "stage_type:[[:space:]]*['\"]\?superflow" "${flow_dir}" >/dev/null 2>&1; then
  fail "unsupported superflow stage detected"
fi

if grep -R "/home/std/\|/Users/\|/home/[^[:space:]]*/project" "${scan_paths[@]}" >/dev/null 2>&1; then
  warn "possible personal absolute path found"
fi

if [[ -f "${flow_dir}/default.yaml" ]]; then
  if grep -nE "model_path|engine_path|onnx_path" "${flow_dir}/default.yaml" >/dev/null 2>&1; then
    info "model path keys found in default.yaml; verify !!path, platform and existence"
    if grep -nE "model_path|engine_path|onnx_path" "${flow_dir}/default.yaml" | grep -v "!!path" >/dev/null 2>&1; then
      warn "model/engine/onnx path may be missing !!path tag"
    fi
  fi

  if grep -nE "^[[:space:]]*-[[:space:]]*[^.]+\.[^.]+\.[^.]+" "${flow_dir}/default.yaml" >/dev/null 2>&1; then
    ok "input-like stage.flow.field refs found"
  else
    info "no stage.flow.field refs found in default.yaml; acceptable for cam or empty input flows"
  fi
fi

printf '\nSummary: %d failure(s), %d warning(s)\n' "${failures}" "${warnings}"
if (( failures > 0 )); then
  exit 1
fi
