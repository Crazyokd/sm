# Flow 阶段与插件规则

## 目录规则

源码目录按依赖边界选择。纯 pipeline Flow 放：

```text
pipeline/src/flow_impls/<序号>.<stage>/<flow_type>/
```

依赖 ROS2 topic、`sensor_msgs`、`cv_bridge`、TF、ROS service 的 adapter Flow 放：

```text
scheduler/adapters/ros2/flow_plugins/<序号>.<stage>/<flow_type>/
```

示例：

```text
pipeline/src/flow_impls/3.frontend/my_detector_flow/
scheduler/adapters/ros2/flow_plugins/1.cam/ros_camera_standard_input_flow/
```

常用序号：

```text
1.cam
2.preprocess
3.frontend
4.backend
5.fusion
6.tracking
7.output
```

## 文件规则

标准 Flow 模板包含：

```text
<flow_type>.hpp
<flow_type>.cpp
default.yaml
README.md
```

如有特殊依赖、额外源文件、TensorRT/Python/第三方库，再加 `CMakeLists.txt`。

所有阶段模板都保持这个结构。`README.md` 用中文说明 Flow 的用途、所属 stage、输入字段、输出字段、关键参数、模型/engine 依赖、最小验证命令和注意事项，方便新同事不用通读代码就能接手。

## 类继承

优先继承 StageFlowBase 的阶段别名：

```cpp
CamFlowBase
PreprocessFlowBase
FrontendFlowBase
BackendFlowBase
FusionFlowBase
TrackingFlowBase
OutputFlowBase
```

必须实现：

```cpp
ExptCode Exec(const PipelineData& input, algo::AlgoPerceptionData& output) override;
std::vector<InputRequirement> GetInputRequirements() const override;
std::vector<OutputRequirement> GetOutputRequirements() const override;
```

需要参数时重写：

```cpp
ExptCode ParseParams(const YAML::Node& params) override;
```

## 导出宏

推荐：

```cpp
EXPORT_STAGE_FLOW_PLUGIN(perception::MyFlow, "my_flow_type")
```

直接继承 `FlowBase` 时才用：

```cpp
EXPORT_FLOW_PLUGIN(perception::MyFlow, "frontend", "my_flow_type")
```

## 插件命名与产物目录

构建产物必须是：

```text
libflow_{stage}_{flow_type}.so
```

`default.yaml` 里的 `type` 必须等于 `flow_type`。

源码可以分层管理，但最终 `.so` 通过插件输出目录统一管理。顶层构建默认输出到仓库根目录 `plugins/`；子仓库单独构建时，pipeline 默认输出到 `pipeline/plugins/`，scheduler 默认输出到 `scheduler/plugins/`。需要统一时使用顶层 `script/build.sh --plugin-dir <dir>` 或环境变量 `PLUGIN_OUTPUT_DIR`。

## 新算法接入原则

- 不改算法核心逻辑。
- 新增 wrapper 负责读取 `PipelineData`、调用算法、把结果写回 `AlgoPerceptionData`。
- 模型加载、阈值、类别表等从 `params` 读取。
- TensorRT/CUDA/Python 资源不可重入时，重写 `IsThreadSafe()` 返回 `false`。
- 有状态计数器、缓存、跨帧状态且不可共享时，重写 `IsShareable()` 返回 `false`。
