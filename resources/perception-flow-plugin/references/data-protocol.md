# 数据协议规则

## 统一数据总线

```cpp
using PipelineData = std::map<std::string, algo::AlgoPerceptionData>;
```

key 是 flow 名，例如 `main_cam`、`resize`、`seg`、`tag_loc`。

## 统一容器

```cpp
struct AlgoPerceptionData {
    std::optional<AlgoSensorInput> sensor_input;
    std::optional<AlgoPerceptionOutput> perception_output;
};
```

## 阶段写入规则

- `cam`、`preprocess`：写 `output.sensor_input`
- `frontend`、`backend`、`fusion`、`tracking`、`output`：写 `output.perception_output`

不要随便同时写两个容器，除非有明确需求并已和用户确认。

## 常用输入字段

`sensor_input` 常见字段：

```text
rgb
depth
aligned_depth
ir
cloud
scan
```

`perception_output` 常见字段：

```text
objects
messages
poses      # 实际仍通常承载在 objects 的 pose6d 字段里，YAML output 可声明 poses
result_image
frame_id
```

## 对象结构

`AlgoPerceptionObject` 是标准目标载体，优先使用已有字段：

```text
category.class_id / class_name / confidence
box2d
mask2d
pose6d
tracking
motion
geometry2d
geometry3d
size
```

不要为单个算法私自发明新的跨 Flow 数据协议。确实需要新增字段时，先解释为什么现有字段不够，并让用户确认。

## 协议层与算法层

- `perception_protocol.hpp`：协议层，轻依赖，适合跨模块传输。
- `algo_types.hpp`：算法层，使用 `cv::Mat` / `pcl::PointCloud`，适合算法内部处理。
- `protocol_converter.hpp`：`toAlgo()` / `toProtocol()` 双向转换。

Flow 插件内部通常使用 `perception::algo::*` 类型。
