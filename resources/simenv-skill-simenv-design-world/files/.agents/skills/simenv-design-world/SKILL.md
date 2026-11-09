---
name: simenv-design-world
description: 从测试/功能需求反推该造一个什么样的世界（SDF 几何 + 待避区/窄道尺寸 + 定位特征 + bundle 里该声明哪些站点与初始位姿），并给出"这个需求必须被几何强制发生"的自检。当用户说"要给某功能造个测试世界""多车解锁/让行/窄道对峙该造什么世界""新需求对应哪个场景和世界""测不了这个场景"时使用。
---

# 从需求造世界

世界不是背景，是**约束**：需求里那个情形必须被几何**强制**发生，不能"碰巧发生"。造之前先写下这句话，再照它选尺寸。

## 1. 需求 → 几何配方（本仓验证过的几种）

| 需求 | 几何配方 | 关键尺寸（0.82 m 车宽） |
|---|---|---|
| 两车窄道互等 / 错身 | **唯一通路**的窄道 + 两端待避区 | 车道净宽 1.0~1.3 m（单车能过、两车不能）；待避区 ≥2.2 m 宽、能停两台 |
| 同目标"谁先走"死锁 | 待避区里并排两车、同向、同目标站点 | 间距 0.4~1.2 m（贴到车长以内才会互顶）|
| 宽道**不该**解锁（负例对照） | 同场景但车道放宽到两车并行 | 净宽 ≥2.4 m |
| 开阔场地定位 / 导航 | 大空间 + 稀疏障碍 + 不对称地标 | 障碍间距 > 激光有效距离的一半 |
| 建图 / 闭环 | 可**环行**的闭合路径 | 环道净宽 ≥ 车宽 + 余量 |

横向错位一定要用**缓变**（例如 0.3 m / 1 m 的斜段）：台阶会把 1.2 m 车道卡成 0.9 m，
车过不去、规划器也拒（这是真踩：corridor 第一版就是台阶）。

## 2. 唯一通路必须自检（最容易翻车的一步）

写完 SDF 先跑：

```bash
python3 ~/.cache/simenv-scenarios/connectivity.py deploy/assets/worlds/<world>/gazebo/world.sdf
```

要看到：起点到终点**可达**、**不该到的区域不可达**、最窄通道（按车半宽膨胀后）留有余量。
真踩的教训：第一版 corridor 把车道墙"立在开阔场地里"，结果车从墙外绕过去
（真值 y≈−1.7，而车道南墙只覆盖 y∈[−0.8,−0.6]）——被测情形根本没发生，日志里还看不到异常。
墙要接到场地外墙上，两端用"封口墙 + 外扩待避区"收口。

## 3. 定位特征要在设计里一次给够

直走廊沿轴向**自相似** → 扫描匹配会沿轴滑动（实测偏 2.2 m，一度被误判成 app 侧问题）。
所以：

- 沿路加**纵向特征**：缓变错位、墙凹口（0.5~0.6 m 深）、不对称地标（一侧方块 / 一侧圆柱）；
- 特征**不要凸进通道**（会刮车、把图开糊）——用**凹口**代替凸柱；
- **待避区本身要有特征**（三面墙 + 一个地标）：初始位姿就放这儿，因为
  **直线车道中段不能用传送摆位**（x 弱可观测 + 传送打乱里程计，实测读数在 −1.4 m ~ +1.2 m 之间乱跳）。

## 4. 世界文件的规矩

- 静态几何放一个 `<model>` 里（`<static>true</static>`），场地**外墙不要动**：换几何本来就要重开图，
  外墙不动才能让 `origin_offset_mm` 的语义稳定；
- 世界名 = bundle 目录名（`deploy/assets/worlds/<world>/`）= manifest 的 `world`；SDF 固定放
  `gazebo/world.sdf`。模型来自 pinned EasyGo 修订（记在
  `deploy/assets/EASYGO_WORLD_SOURCE_SHA`，换 SHA 同步镜像 LABEL）；本仓自造的几何要在 manifest 里
  写明 `source_sha` 与理由；
- 改完必须 `docker build -f deploy/Dockerfile -t simenv-world-gazebo:local .`（世界打进镜像）；
- 命名/端口/坐标口径按 `AGENTS.md` 的"契约"一节（`robotN`、`ROS_DOMAIN_ID=11+(N-1)`、`frontend=18088+100N`、
  `api=18089+100N`）。

## 5. bundle 里该声明什么（这个世界对应的"一张图 + 可定位坐标"）

写 `deploy/assets/worlds/<world>/bundle.json`（格式与逐字段语义见 skill `simenv-build-map` §5）：

- `stations[]`：**只放车真会停的位置**（**世界系 m/rad**），且必须落在地图**观测过**的区域内；
- `initial_poses[]`：世界位姿，**给就要给全**（缺一台 `simenv start` 直接失败；与 `--poses` 同给也失败），
  放**有特征处**，朝向朝任务方向（车先动起来比原地转更稳）；
- `vehicles.easygo` 节的 `origin_offset_mm`：开图后**实测**写入（开图流程见 skill `simenv-build-map`）；
- scenarios 不进 bundle：场景组归 harness runner，一张图对应一组坐标即可。

## 6. 迭代法与收口

1. 写/改 SDF → `connectivity.py` 自检 → `docker build`；
2. 走 **`simenv-build-map`** 开图 + 量偏移 + 写 manifest；
3. `go test ./...`（资产校验，含"声明坐标必须落在观测过的区域"）；
4. `simenv start --world <world> --env-image simenv-world-gazebo:local --app-image …` 退出 0 且
   两车位姿差在容差内 → 跑 harness 场景组；
5. **几何一改就要重开图**（地图绑定世界），所以尺寸尽量一次算对。

## 7. 别让世界的锅背错

场景红了先分三层定位，写清楚归属：

- **世界层**：连通性/尺寸（可绕行、过不去、台阶卡宽）——本 skill 的自检抓；
- **接线层**：坐标、站点、初始位姿、地图是否为这张图、偏移对不对——bundle 的校验 + `measure_offset` 抓；
- **其它层**：app 的多车逻辑、规划器、定位阈值——用日志签名归因（例如协调车在
  `CONTINUE_TASK` ↔ `DRIVE_ON_HEADING_BACKWARD` 之间振荡导致"最终消解"不完成，属 app 侧）。
  世界设计要做的是**让被测情形一定发生**，而不是替其它层兜底。

## 相关

- 开图与 bundle：skill `simenv-build-map`；起停：skill `simenv-run`；
- 合同：`docs/superpowers/specs/2026-11-05-ports-architecture.md`（§5 bundle 格式）；
- 工具：`~/.cache/simenv-scenarios/connectivity.py` 等（见 `simenv-build-map`）。
