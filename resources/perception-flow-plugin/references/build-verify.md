# Flow 构建验证规则

## 目标

新建或修改 Flow 后，必须确认它能按感知仓库现有构建体系生成插件 `.so`。

标准产物命名：

```text
libflow_<stage>_<flow_type>.so
```

常见输出目录：

```text
plugins/
pipeline/plugins/
scheduler/plugins/
```

示例：

```text
plugins/libflow_frontend_demo_frontend_flow.so
plugins/libflow_cam_ros_cam_flow.so
```

## 推荐命令

```bash
# 在感知仓库根目录执行
bash .agents/skills/perception-flow-plugin/scripts/verify_flow_build.sh <flow_dir>
```

带并行参数：

```bash
bash .agents/skills/perception-flow-plugin/scripts/verify_flow_build.sh <flow_dir> -j 8
```

## 脚本做什么

1. 从目录推导 stage 和 flow_type。
2. 运行 `inspect_flow_plugin.sh` 做静态体检。
3. 根据源码位置选择构建链：`pipeline/src/flow_impls` 调用 `pipeline/build.sh build --release`；`scheduler/adapters/ros2/flow_plugins` 调用 `script/build.sh --build --ros2 --release`。
4. 检查插件输出目录里是否存在 `libflow_<stage>_<flow_type>.so`。
5. 找不到产物时输出可能原因。

## 轻量运行时加载验证

构建产物存在后，可运行：

```bash
bash .agents/skills/perception-flow-plugin/scripts/verify_plugin_load.sh <flow_dir> --plugin-dir "$(pwd)/plugins"
```

脚本会检查：

1. `.so` 文件存在。
2. 同目录默认配置 `flow_<stage>_<flow_type>.yaml` 存在。
3. 导出符号 `create_flow`、`destroy_flow`、`get_flow_type`、`get_flow_stage` 存在。
4. 临时 C++ loader 能 `dlopen` 插件。
5. `create_flow()` 能创建实例。
6. 默认 YAML 能加载。
7. `FlowBase::Init()` 能完成通用字段和 `params` 解析。

它不调用 `Exec()`，因此适合在没有真实 ROS topic、相机、模型输入的开发机上快速确认插件运行时入口。

## 常见失败原因

- 目录不在 `pipeline/src/flow_impls/<序号>.<stage>/<flow_type>/` 或 `scheduler/adapters/ros2/flow_plugins/<序号>.<stage>/<flow_type>/`。
- 目录里没有 `.cpp` 源文件。
- 自定义 `CMakeLists.txt` 没有调用 `add_flow_plugin()`。
- `default.yaml type`、目录名、导出宏 flow_type 不一致。
- 插件依赖缺失，CMake 或链接失败。
- TensorRT / Python / 模型相关依赖没有在当前开发机准备好。
- ROS2 adapter Flow 构建前没有 source ROS2 环境，或没有使用 `--ros2` 构建链。

## 何时用顶层构建

如果改动跨越 `infra`、`pipeline`、`app` 或依赖顶层构建选项，再使用仓库顶层构建脚本：

```bash
# 在感知仓库根目录执行
bash script/build.sh --build
```

只改单个 pipeline Flow 时，`verify_flow_build.sh` 会走更快的 pipeline 子构建；ROS2 adapter Flow 会走顶层 ROS2 构建链，保证 ROS2 依赖和统一插件输出目录正确。
