---
name: perception-flow-plugin
description: 为感知仓库 pipeline 框架开发、拆分、接入和体检 Flow 插件。适用于 Codex 需要创建标准 Flow 骨架、判断应该创建哪个阶段的 Flow、把已有算法按感知数据协议拆成多阶段 Flow、接入 pipeline YAML、检查输入输出合法性、检查模型/engine/配置路径是否合理，或处理用户提到 flow、pipeline、插件、frontend/backend/output、superflow、算法接入等任务。
---

# 感知 Flow 插件

这个 skill 用来帮助 Codex 在感知仓库里按现有 pipeline 架构开发和检查 Flow 插件。默认先读项目已有文件，再给方案；复杂算法接入必须先和用户确认拆分方案，再改代码。

## 必读原则

- 先读现有同阶段 demo，再创建或修改插件。
- 不重写算法核心逻辑。已有算法只做 wrapper、参数读取、数据转换和 pipeline 接入。
- 优先拆成标准阶段 Flow，不鼓励为了省事把所有逻辑塞进一个 Flow。
- 当前框架不支持独立 `superflow` stage。只能创建“复合型普通 Flow”，挂在最接近输出语义的现有 stage。
- 所有路径、命令和验证都基于当前感知仓库根目录，不硬编码个人路径。
- 源码目录按依赖边界分层：纯 pipeline Flow 放 `pipeline/src/flow_impls`；依赖 ROS2 topic、`sensor_msgs`、`cv_bridge`、TF、ROS service 的 Flow 放 `scheduler/adapters/ros2/flow_plugins`。
- 不管源码来自哪个 Flow 目录，最终插件产物都按 `PLUGIN_OUTPUT_DIR` / `--plugin-dir` 统一输出和加载。
- 修改前说明目标文件；新建或修改 Flow 后必须完成静态体检、构建验证和 `.so` 产物检查，除非用户明确要求只讨论不改代码。

## 何时读取参考资料

- 需要解释整体架构或判断阶段：读 `references/pipeline-architecture.md`。
- 需要写插件代码或目录：读 `references/flow-stage-rules.md`。
- 需要处理数据结构、输入输出字段：读 `references/data-protocol.md`。
- 需要写 pipeline YAML、模型路径、engine 配置：读 `references/yaml-model-rules.md`。
- 需要体检已有 Flow：读 `references/health-check.md`，并优先运行 `scripts/inspect_flow_plugin.sh`。
- 需要验证新建/修改的 Flow 能否编译：读 `references/build-verify.md`，并运行 `scripts/verify_flow_build.sh`。
- 需要验证已生成 `.so` 能否被运行时加载和初始化：运行 `scripts/verify_plugin_load.sh`。

## 标准工作流

1. 明确用户目标：新建 Flow、改造已有算法、检查已有 Flow、还是接入 pipeline YAML。
2. 判断 stage：
   - 采集图像/点云/ROS topic：`cam`
   - resize、归一化、裁剪、畸变处理：`preprocess`
   - 检测、分割、分类、关键点：`frontend`
   - 位姿估计、3D 定位、几何求解：`backend`
   - 多相机、多模型、多传感器结果融合：`fusion`
   - ID 关联、时序跟踪、状态维护：`tracking`
   - ROS 发布、TF、保存结果、可视化、最终输出：`output`
3. 如果算法横跨多个阶段，先给拆分方案：每个阶段的 flow_type、输入字段、输出字段、复用的算法类/函数、YAML 接线方式、验证命令。
4. 创建或修改插件时，先判断源码目录：纯 pipeline Flow 用 `pipeline/src/flow_impls/<序号>.<stage>/<flow_type>/`；ROS2 adapter Flow 用 `scheduler/adapters/ros2/flow_plugins/<序号>.<stage>/<flow_type>/`。
5. 每个 Flow 至少包含 `.hpp`、`.cpp`、`default.yaml`、`README.md`；特殊依赖再加 `CMakeLists.txt`。所有阶段模板都遵循这个文件结构。
6. 代码推荐继承 `CamFlowBase` / `PreprocessFlowBase` / `FrontendFlowBase` / `BackendFlowBase` / `FusionFlowBase` / `TrackingFlowBase` / `OutputFlowBase`。
7. 使用 `EXPORT_STAGE_FLOW_PLUGIN(perception::ClassName, "flow_type")` 导出插件。
8. 更新或新增 pipeline YAML，确保 `input` 是 `stage.flow.field` 格式，且只引用前序 stage。
9. 新建或修改 Flow 后，运行 `inspect_flow_plugin.sh` 做静态体检。
10. 运行 `verify_flow_build.sh` 编译对应构建链，并确认 `libflow_<stage>_<flow_type>.so` 在插件输出目录生成。
11. 运行 `verify_plugin_load.sh` 对生成的 `.so` 做轻量运行时验证：`dlopen`、导出符号、默认 YAML、`FlowBase::Init()`。
12. 如果新增或修改 pipeline YAML，再运行最小 pipeline 初始化/测试。

## Superflow 规则

如果用户提出“superflow”、“全阶段 flow”、“一个 flow 直接输入到输出”：

- 说明当前框架不支持独立 `superflow` stage。
- 如果用户只是快速验证，可以创建“复合型普通 Flow”：
  - 图像直接产出检测对象：挂 `frontend`
  - 检测对象 + RGBD 产出 6D 位姿：挂 `backend`
  - 多个结果汇总/发布/保存：挂 `output`
- 长期维护场景优先拆成多阶段标准 Flow。
- 不要创建 `stage_type: superflow`、`libflow_superflow_*.so` 或新的 StageType，除非用户明确要求修改框架。

## 构建闭环

新建或修改 Flow 后，不要只停在代码生成。必须确认插件能按正常感知架构编译成 `.so`：

```bash
# 在感知仓库根目录执行
bash .agents/skills/perception-flow-plugin/scripts/verify_flow_build.sh pipeline/src/flow_impls/3.frontend/demo_frontend_flow
```

这个脚本会先静态体检，再按源码位置选择构建命令：pipeline Flow 调用 `pipeline/build.sh`；ROS2 adapter Flow 调用 `script/build.sh --build --ros2`；最后检查目标插件产物。

## 体检入口

```bash
# 在感知仓库根目录执行
bash .agents/skills/perception-flow-plugin/scripts/inspect_flow_plugin.sh pipeline/src/flow_impls/3.frontend/demo_frontend_flow
```

静态体检只是初筛，不能替代构建和运行。新建或修改 Flow 时优先使用 `verify_flow_build.sh` 完成闭环。

## 轻量加载验证

当 `.so` 已经生成，但不想重复跑完整构建时，用：

```bash
# 在感知仓库根目录执行
bash .agents/skills/perception-flow-plugin/scripts/verify_plugin_load.sh \
  scheduler/adapters/ros2/flow_plugins/1.cam/ros_cam_flow \
  --plugin-dir "$(pwd)/plugins"
```

这个脚本会临时编译一个 loader，检查 `.so` 是否能 `dlopen`、是否导出 `create_flow` / `destroy_flow` / `get_flow_type` / `get_flow_stage`、同目录默认 YAML 是否存在，并调用 `FlowBase::Init()` 验证参数解析。它不调用 `Exec()`，因此不会等待真实相机、模型推理或 ROS topic 数据。
