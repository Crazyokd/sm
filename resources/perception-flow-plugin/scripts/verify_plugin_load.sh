#!/usr/bin/env bash
# 功能：轻量验证感知 Flow 插件 .so 能被运行时加载并初始化。
# 具体做什么：解析 flow 目录或 .so 路径，检查导出符号、默认 YAML 配置，临时编译一个 loader 调用 create_flow/get_flow_type/get_flow_stage，并执行 FlowBase::Init()。
# 使用方法：bash .agents/skills/perception-flow-plugin/scripts/verify_plugin_load.sh scheduler/adapters/ros2/flow_plugins/1.cam/ros_cam_flow --plugin-dir "$(pwd)/plugins"
# 运行位置：在感知仓库根目录执行；脚本只在 /tmp 生成临时编译文件，不修改源码。
set -euo pipefail

target=""
plugin_dir=""
flow_name="load_check"
stage_name=""
stage_type=""
keep_tmp=false
load_timeout_s=20

info() { printf '[INFO] %s\n' "$*"; }
ok() { printf '[OK]   %s\n' "$*"; }
fail() { printf '[FAIL] %s\n' "$*" >&2; }

usage() {
  cat <<USAGE
Usage:
  ${0##*/} <flow_dir|plugin.so> [--plugin-dir DIR] [--flow-name NAME] [--stage-name NAME] [--stage-type STAGE] [--timeout SEC] [--keep-tmp]

Examples:
  # 从 Flow 源码目录推导插件路径
  bash .agents/skills/perception-flow-plugin/scripts/verify_plugin_load.sh \\
    scheduler/adapters/ros2/flow_plugins/1.cam/ros_orbbec_dcw2_cam_flow \\
    --plugin-dir "\$(pwd)/plugins"

  # 直接验证 .so
  bash .agents/skills/perception-flow-plugin/scripts/verify_plugin_load.sh \\
    plugins/libflow_cam_ros_orbbec_dcw2_cam_flow.so
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --plugin-dir)
      [[ $# -ge 2 ]] || { fail "Missing value for --plugin-dir"; exit 2; }
      plugin_dir="$2"
      shift 2
      ;;
    --flow-name)
      [[ $# -ge 2 ]] || { fail "Missing value for --flow-name"; exit 2; }
      flow_name="$2"
      shift 2
      ;;
    --stage-name)
      [[ $# -ge 2 ]] || { fail "Missing value for --stage-name"; exit 2; }
      stage_name="$2"
      shift 2
      ;;
    --stage-type)
      [[ $# -ge 2 ]] || { fail "Missing value for --stage-type"; exit 2; }
      stage_type="$2"
      shift 2
      ;;
    --keep-tmp)
      keep_tmp=true
      shift
      ;;
    --timeout)
      [[ $# -ge 2 ]] || { fail "Missing value for --timeout"; exit 2; }
      load_timeout_s="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      fail "Unknown option: $1"
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "${target}" ]]; then
        fail "Only one target is allowed"
        exit 2
      fi
      target="$1"
      shift
      ;;
  esac
done

if [[ -z "${target}" ]]; then
  usage >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "${repo_root}"

target="${target%/}"
so_path=""
flow_type=""
stage=""

if [[ -d "${target}" ]]; then
  flow_type="$(basename "${target}")"
  stage_dir="$(basename "$(dirname "${target}")")"
  stage="${stage_dir#*.}"
  expected_so="libflow_${stage}_${flow_type}.so"

  if [[ -z "${plugin_dir}" ]]; then
    case "${target}" in
      pipeline/src/flow_impls/*) plugin_dir="pipeline/plugins" ;;
      scheduler/adapters/ros2/flow_plugins/*) plugin_dir="plugins" ;;
      *) plugin_dir="plugins" ;;
    esac
  fi

  so_path="${plugin_dir%/}/${expected_so}"
elif [[ -f "${target}" ]]; then
  so_path="${target}"
  filename="$(basename "${so_path}")"
  if [[ ! "${filename}" =~ ^libflow_([^_]+)_(.+)\.so$ ]]; then
    fail "Invalid plugin filename: ${filename}; expected libflow_<stage>_<flow_type>.so"
    exit 1
  fi
  stage="${BASH_REMATCH[1]}"
  flow_type="${BASH_REMATCH[2]}"
else
  fail "Target not found: ${target}"
  exit 1
fi

case "${stage}" in
  cam|preprocess|frontend|backend|fusion|tracking|output)
    ;;
  *)
    fail "Invalid stage '${stage}'"
    exit 1
    ;;
esac

if [[ -z "${stage_type}" ]]; then
  stage_type="${stage}"
fi
if [[ -z "${stage_name}" ]]; then
  stage_name="${stage}"
fi

if [[ ! -f "${so_path}" ]]; then
  fail "Plugin .so not found: ${so_path}"
  exit 1
fi
so_path="$(realpath "${so_path}")"
ok "plugin exists: ${so_path}"

default_yaml="$(dirname "${so_path}")/flow_${stage}_${flow_type}.yaml"
if [[ ! -f "${default_yaml}" ]]; then
  fail "Default config not found: ${default_yaml}"
  exit 1
fi
ok "default config exists: ${default_yaml}"

missing_symbols=0
for symbol in create_flow destroy_flow get_flow_type get_flow_stage; do
  if nm -D "${so_path}" 2>/dev/null | grep -E "[[:space:]]${symbol}$" >/dev/null; then
    ok "export symbol found: ${symbol}"
  else
    fail "export symbol missing: ${symbol}"
    missing_symbols=$((missing_symbols + 1))
  fi
done
if (( missing_symbols > 0 )); then
  exit 1
fi

tmp_dir="$(mktemp -d /tmp/perception_plugin_load.XXXXXX)"
if [[ "${keep_tmp}" == false ]]; then
  trap 'rm -rf "${tmp_dir}"' EXIT
else
  info "keeping tmp dir: ${tmp_dir}"
fi

cat > "${tmp_dir}/verify_plugin_load.cpp" <<'CPP'
#include <dlfcn.h>

#include <iostream>
#include <memory>
#include <string>

#include <yaml-cpp/yaml.h>

#include "core/flow_base.hpp"
#include "exception_code.hpp"
#include "yaml_loader/yaml_loader.hpp"

namespace
{

using CreateFlowFn = perception::FlowBase* (*)();
using DestroyFlowFn = void (*)(perception::FlowBase*);
using StringFn = const char* (*)();

template <typename T>
T load_symbol(void* handle, const char* name)
{
    dlerror();
    auto symbol = reinterpret_cast<T>(dlsym(handle, name));
    const char* error = dlerror();
    if (error != nullptr || symbol == nullptr)
    {
        std::cerr << "[FAIL] dlsym " << name << ": " << (error ? error : "null") << '\n';
        return nullptr;
    }
    std::cout << "[OK]   dlsym " << name << '\n';
    return symbol;
}

}  // namespace

int main(int argc, char** argv)
{
    if (argc != 6)
    {
        std::cerr << "Usage: verify_plugin_load <so> <yaml> <flow_name> <stage_name> <stage_type>\n";
        return 2;
    }

    const std::string so_path = argv[1];
    const std::string yaml_path = argv[2];
    const std::string flow_name = argv[3];
    const std::string stage_name = argv[4];
    const std::string stage_type = argv[5];

    void* handle = dlopen(so_path.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (!handle)
    {
        std::cerr << "[FAIL] dlopen: " << dlerror() << '\n';
        return 1;
    }
    std::cout << "[OK]   dlopen: " << so_path << '\n';

    auto create_flow = load_symbol<CreateFlowFn>(handle, "create_flow");
    auto destroy_flow = load_symbol<DestroyFlowFn>(handle, "destroy_flow");
    auto get_flow_type = load_symbol<StringFn>(handle, "get_flow_type");
    auto get_flow_stage = load_symbol<StringFn>(handle, "get_flow_stage");
    if (!create_flow || !destroy_flow || !get_flow_type || !get_flow_stage)
    {
        dlclose(handle);
        return 1;
    }

    const std::string plugin_type = get_flow_type();
    const std::string plugin_stage = get_flow_stage();
    std::cout << "[OK]   plugin declaration: " << plugin_stage << "/" << plugin_type << '\n';

    std::unique_ptr<perception::FlowBase, DestroyFlowFn> flow(create_flow(), destroy_flow);
    if (!flow)
    {
        std::cerr << "[FAIL] create_flow returned null\n";
        dlclose(handle);
        return 1;
    }
    std::cout << "[OK]   create_flow\n";

    YAML::Node config;
    try
    {
        config = YAML::LoadFileWithInclude(yaml_path);
    }
    catch (const std::exception& e)
    {
        std::cerr << "[FAIL] load default yaml: " << e.what() << '\n';
        dlclose(handle);
        return 1;
    }
    std::cout << "[OK]   load default yaml: " << yaml_path << '\n';

    auto code = flow->Init(flow_name, stage_name, stage_type, config);
    if (code != perception::expt_pipeline_no_error)
    {
        std::cerr << "[FAIL] FlowBase::Init returned: 0x" << std::hex << code << std::dec << '\n';
        dlclose(handle);
        return 1;
    }
    std::cout << "[OK]   FlowBase::Init\n";

    std::cout << "[OK]   flow name: " << flow->Name() << '\n';
    std::cout << "[OK]   output requirements: " << flow->GetOutputRequirements().size() << '\n';

    flow.reset();
    dlclose(handle);
    return 0;
}
CPP

cat > "${tmp_dir}/CMakeLists.txt" <<CPP
cmake_minimum_required(VERSION 3.16)
project(perception_verify_plugin_load LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(REPO_ROOT "${repo_root}")

find_package(yaml-cpp REQUIRED)
find_package(OpenCV REQUIRED)
find_package(PCL REQUIRED COMPONENTS common)
find_package(Eigen3 REQUIRED)

add_executable(verify_plugin_load verify_plugin_load.cpp)
target_include_directories(verify_plugin_load PRIVATE
    "\${REPO_ROOT}/pipeline/src"
    "\${REPO_ROOT}/infra"
    "\${REPO_ROOT}/infra/exception"
    "\${REPO_ROOT}/infra/yaml_loader"
    "\${REPO_ROOT}/infra/unified_log"
    "\${REPO_ROOT}/infra/blackboard"
    \${OpenCV_INCLUDE_DIRS}
    \${PCL_INCLUDE_DIRS}
    \${EIGEN3_INCLUDE_DIRS}
)
target_link_libraries(verify_plugin_load PRIVATE
    "\${REPO_ROOT}/pipeline/build/lib/libpipeline.so"
    "\${REPO_ROOT}/infra/build/lib/libyaml_loader_lib.so"
    yaml-cpp
    dl
)
target_compile_definitions(verify_plugin_load PRIVATE
    PROJECT_NAME="verify_plugin_load"
    PROJECT_VERSION_FULL="0.0.0"
)
CPP

info "building temporary loader"
cmake -S "${tmp_dir}" -B "${tmp_dir}/build" >/dev/null
cmake --build "${tmp_dir}/build" -j 2 >/dev/null
ok "temporary loader built"

info "running plugin load check"
mkdir -p "${tmp_dir}/ros_log"
timeout "${load_timeout_s}" env \
  ROS_LOG_DIR="${tmp_dir}/ros_log" \
  RCUTILS_LOGGING_USE_STDOUT=1 \
  "${tmp_dir}/build/verify_plugin_load" "${so_path}" "${default_yaml}" "${flow_name}" "${stage_name}" "${stage_type}"
