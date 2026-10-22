#!/usr/bin/env bash
# 功能：验证感知 Flow 插件能按正常仓库架构编译并生成 .so。
# 具体做什么：先运行静态体检，再根据源码位置选择 pipeline 构建或 ROS2 顶层构建，最后检查 libflow_<stage>_<flow_type>.so 是否存在。
# 使用方法：bash .agents/skills/perception-flow-plugin/scripts/verify_flow_build.sh <flow_dir> [-j N] [--debug|--release] [--plugin-dir DIR]
# 运行位置：在感知仓库根目录执行；脚本不修改源码，只触发构建产物生成。
set -euo pipefail

flow_dir=""
build_type="--release"
jobs=""
plugin_dir=""

usage() {
  cat <<USAGE
Usage:
  ${0##*/} <flow_dir> [-j N] [--debug|--release] [--plugin-dir DIR] [--skip-inspect]

Options:
  -j N            Build jobs passed to build script.
  --debug         Build Debug.
  --release       Build Release. Default.
  --plugin-dir    Plugin output directory. Defaults to pipeline/plugins for pipeline Flow, plugins for ROS2 adapter Flow.
  --skip-inspect  Skip static inspect step.

Examples:
  # Pipeline Flow
  bash .agents/skills/perception-flow-plugin/scripts/verify_flow_build.sh pipeline/src/flow_impls/3.frontend/demo_frontend_flow -j 8

  # ROS2 adapter Flow
  bash .agents/skills/perception-flow-plugin/scripts/verify_flow_build.sh scheduler/adapters/ros2/flow_plugins/1.cam/ros_cam_flow -j 8
USAGE
}

skip_inspect=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    -j)
      [[ $# -ge 2 ]] || { echo "[ERROR] Missing value for -j" >&2; exit 2; }
      jobs="$2"
      shift 2
      ;;
    -j*)
      jobs="${1#-j}"
      shift
      ;;
    --plugin-dir)
      [[ $# -ge 2 ]] || { echo "[ERROR] Missing value for --plugin-dir" >&2; exit 2; }
      plugin_dir="$2"
      shift 2
      ;;
    --debug)
      build_type="--debug"
      shift
      ;;
    --release)
      build_type="--release"
      shift
      ;;
    --skip-inspect)
      skip_inspect=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -* )
      echo "[ERROR] Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "${flow_dir}" ]]; then
        echo "[ERROR] Only one flow_dir is allowed" >&2
        exit 2
      fi
      flow_dir="$1"
      shift
      ;;
  esac
done

if [[ -z "${flow_dir}" ]]; then
  usage >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "${repo_root}"

if [[ ! -d "${flow_dir}" ]]; then
  echo "[ERROR] Flow directory not found: ${flow_dir}" >&2
  exit 1
fi

flow_dir="${flow_dir%/}"
flow_type="$(basename "${flow_dir}")"
stage_dir="$(basename "$(dirname "${flow_dir}")")"
stage="${stage_dir#*.}"
expected_so="libflow_${stage}_${flow_type}.so"
inspect_script=".agents/skills/perception-flow-plugin/scripts/inspect_flow_plugin.sh"

case "${stage}" in
  cam|preprocess|frontend|backend|fusion|tracking|output)
    ;;
  *)
    echo "[ERROR] Invalid stage '${stage}' from ${flow_dir}" >&2
    exit 1
    ;;
esac

source_kind=""
default_plugin_dir=""
case "${flow_dir}" in
  pipeline/src/flow_impls/*)
    source_kind="pipeline"
    default_plugin_dir="pipeline/plugins"
    ;;
  scheduler/adapters/ros2/flow_plugins/*)
    source_kind="ros2_scheduler"
    default_plugin_dir="plugins"
    ;;
  *)
    echo "[ERROR] Unsupported Flow source directory: ${flow_dir}" >&2
    echo "Expected pipeline/src/flow_impls/... or scheduler/adapters/ros2/flow_plugins/..." >&2
    exit 1
    ;;
esac

artifact_dir="${plugin_dir:-${PLUGIN_OUTPUT_DIR:-${default_plugin_dir}}}"
expected_path="${artifact_dir}/${expected_so}"

if [[ "${skip_inspect}" == false ]]; then
  echo "==> [verify_flow_build] Static inspect: ${flow_dir}"
  bash "${inspect_script}" "${flow_dir}"
fi

build_args=(build "${build_type}")
if [[ -n "${jobs}" ]]; then
  build_args+=(-j "${jobs}")
fi

case "${source_kind}" in
  pipeline)
    if [[ ! -x pipeline/build.sh ]]; then
      echo "[ERROR] pipeline/build.sh not found or not executable" >&2
      exit 1
    fi
    echo "==> [verify_flow_build] Build pipeline Flow: PLUGIN_OUTPUT_DIR=${artifact_dir} bash pipeline/build.sh ${build_args[*]}"
    PLUGIN_OUTPUT_DIR="${artifact_dir}" bash pipeline/build.sh "${build_args[@]}"
    ;;
  ros2_scheduler)
    if [[ ! -x script/build.sh ]]; then
      echo "[ERROR] script/build.sh not found or not executable" >&2
      exit 1
    fi
    top_args=(--build "${build_type}" --ros2 --plugin-dir "${artifact_dir}")
    if [[ -n "${jobs}" ]]; then
      top_args+=(-j "${jobs}")
    fi
    echo "==> [verify_flow_build] Build ROS2 adapter Flow: bash script/build.sh ${top_args[*]}"
    bash script/build.sh "${top_args[@]}"
    ;;
esac

echo "==> [verify_flow_build] Check plugin artifact: ${expected_so}"
if [[ -f "${expected_path}" ]]; then
  echo "[OK] Found ${expected_path}"
  exit 0
fi

found=""
for search_root in "${artifact_dir}" plugins pipeline/plugins scheduler/plugins; do
  [[ -d "${search_root}" ]] || continue
  found="$(find "${search_root}" -maxdepth 1 -name "${expected_so}" -print -quit 2>/dev/null || true)"
  [[ -n "${found}" ]] && break
done

if [[ -n "${found}" ]]; then
  echo "[OK] Found ${found}"
  exit 0
fi

echo "[ERROR] Expected plugin artifact not found: ${expected_so}" >&2
echo "Possible causes:" >&2
echo "  - Flow directory was not discovered by its flow_plugins/flow_impls CMakeLists.txt" >&2
echo "  - Directory has no .cpp source file" >&2
echo "  - Custom CMakeLists.txt did not call add_flow_plugin with matching stage/type" >&2
echo "  - Export macro or flow_type does not match directory/default.yaml" >&2
echo "  - ROS2 adapter Flow was built without --ros2 or ROS2 environment was not sourced" >&2
echo "  - Build failed before producing the plugin" >&2
exit 1
