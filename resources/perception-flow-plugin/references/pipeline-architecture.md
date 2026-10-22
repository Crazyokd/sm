# Pipeline 架构速记

## 三层结构

```text
Pipeline -> Stage -> Flow 插件 .so
```

Pipeline 按 YAML `stages` 顺序执行。Stage 持有多个 Flow。Flow 是最小算法单元，运行时通过 `.so` 动态加载。

## 固定阶段

| 顺序 | Stage | 主要职责 | 主要输出容器 |
|---|---|---|---|
| 1 | `cam` | 相机/传感器采集、外部输入注入 | `sensor_input` |
| 2 | `preprocess` | resize、归一化、裁剪、图像预处理 | `sensor_input` |
| 3 | `frontend` | 检测、分割、分类、关键点 | `perception_output` |
| 4 | `backend` | 6D 位姿、3D 定位、几何求解 | `perception_output` |
| 5 | `fusion` | 多源融合 | `perception_output` |
| 6 | `tracking` | 目标跟踪、状态维护 | `perception_output` |
| 7 | `output` | 发布、保存、可视化、最终输出 | `perception_output` |

## 处理模式

- `priority`：按顺序执行，首个成功后跳过后续 flow。适合多备选算法。
- `parallel`：执行该 stage 下所有 flow。适合多个独立输出或并行结果。

## 输入引用

YAML 输入引用格式固定为：

```text
stage.flow.field
```

只能引用当前 stage 之前的 stage。跨 stage 读取合法，反向引用不合法。

## Superflow 边界

当前代码只支持上述 7 个 `stage_type`，没有独立 `superflow`。如果需要“一步到位”，做复合型普通 Flow，并挂到最接近输出语义的 stage。
