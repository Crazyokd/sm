# Flow 体检清单

## 目录与命名

- 路径是否是 `pipeline/src/flow_impls/<序号>.<stage>/<flow_type>/`
- stage 是否是固定 7 阶段之一。
- flow_type、目录名、`default.yaml` 的 `type` 是否一致。
- 构建产物是否符合 `libflow_{stage}_{flow_type}.so`。

## 代码

- 是否继承正确阶段基类。
- 是否实现 `Exec()`、`GetInputRequirements()`、`GetOutputRequirements()`。
- 是否使用 `EXPORT_STAGE_FLOW_PLUGIN` 或必要时 `EXPORT_FLOW_PLUGIN`。
- `GetInputRequirements()` 的 required 输入能否在 YAML `input` 中找到。
- `GetOutputRequirements()` 是否和实际输出字段、`default.yaml output` 对齐。
- `cam/preprocess` 是否写 `sensor_input`，后续阶段是否写 `perception_output`。
- 是否不小心修改了算法核心逻辑，而不是做 wrapper。
- 是否需要 `IsThreadSafe()` / `IsShareable()`。

## YAML

- `input` 是否全是 `stage.flow.field`。
- 是否引用前序 stage。
- `plugin_dirs` 是否合理。
- `plugin_path` 是否包含完整 `.so` 文件名。
- `!!path` / `!!include` 是否使用正确。
- 是否出现 `stage_type: superflow` 等框架不支持的写法。

## 模型与平台

- 模型路径是否硬编码个人目录。
- `.engine` 是否匹配运行架构。
- 是否明确 x86/ARM/Jetson/EasyGo 差异。
- 模型文件不存在时是否有清晰报错。
- 参数默认值是否安全。

## 验证建议

优先运行：

```bash
# 在感知仓库根目录执行
bash .agents/skills/perception-flow-plugin/scripts/inspect_flow_plugin.sh <flow_dir>
```

然后运行构建验证：

```bash
# 在感知仓库根目录执行
bash .agents/skills/perception-flow-plugin/scripts/verify_flow_build.sh <flow_dir>
```

如果 `.so` 已生成，需要验证运行时加载和初始化：

```bash
# 在感知仓库根目录执行
bash .agents/skills/perception-flow-plugin/scripts/verify_plugin_load.sh <flow_dir> --plugin-dir "$(pwd)/plugins"

# 或直接传 .so
bash .agents/skills/perception-flow-plugin/scripts/verify_plugin_load.sh plugins/libflow_cam_ros_cam_flow.so
```

不要把静态体检当成最终验证。新建或修改 Flow 时，必须确认 `.so` 产物生成，并尽量运行轻量加载验证。
