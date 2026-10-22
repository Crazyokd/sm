# YAML 与模型配置规则

## Pipeline YAML 基本结构

```yaml
plugin_dirs:
  - !!path "../plugins"

stages:
  - cam
  - preprocess
  - frontend

frontend:
  stage_type: "frontend"
  process_mode: "parallel"
  flows:
    seg:
      type: "my_detector_flow"
      input:
        - preprocess.resize.rgb
      params:
        model_path: !!path "../models/my_model.engine"
      output:
        - objects
```

## 路径规则

- 相对路径使用 `!!path`，基于当前 YAML 文件所在目录解析。
- 不写死个人路径，例如 `/home/std/...`。
- 部署路径可以写 `/opt/perception_platform/...`，但要说明目标环境。
- 本地开发路径应尽量基于仓库相对路径或配置传入。

## 模型配置体检

检查 `params` 中的模型相关项：

- `model_path`、`engine_path`、`onnx_path` 是否存在或是否有部署说明。
- TensorRT `.engine` 是否区分 x86 / ARM / Jetson / EasyGo 设备。
- `.engine` 不要跨架构混用。
- 类别表 `classes` 是否和模型输出类别 ID 对齐。
- 阈值如 `conf_thresh`、`nms_thresh` 是否有默认值。
- Python 模型插件是否说明 Python 环境、脚本路径和依赖。

## default.yaml 规则

每个插件必须有默认配置，安装到插件目录后命名为：

```text
flow_{stage}_{flow_type}.yaml
```

内容至少包含：

```yaml
input: []
type: "flow_type"
params: ~
output: []
```

默认配置会和用户 YAML 深度合并，ResourceKey 也基于完整配置生成。
