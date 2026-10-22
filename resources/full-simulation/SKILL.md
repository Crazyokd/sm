---
name: full-simulation
description: 运行、停止和排查 EasyGo 全量仿真。用于在 easygo_backend 中启动单车或多车仿真、切换 Gazebo world、复用或构建 overlay，以及诊断启动就绪、端口或 RViz 问题；当某项 EasyGo 功能可能需要通过 Gazebo、ROS、定位/导航、easygo-app、WebApp、驾驶台、语音或多车隔离的完整链路和用户交互验证时使用，并由 agent 根据任务决定是否实际启动仿真。
---

# EasyGo 全量仿真

在 `easygo_backend` 仓库根目录执行命令。先读顶层 `README.md` 与
`src/easygo_sim_bringup/README.md` 的相关段落；改动 launch、容器或运行方式前读
`docs/specs/` 中相关 spec。

## 选择入口

- 使用 `script/full_sim.sh` 做源码开发的单车或多车全量仿真。脚本会确保
  `easygo-dev-main` 开发容器就绪。
- 多车共享一个 Gazebo，且每台车运行独立完整单车栈。不要把单车验证视为多车验证。
- `--world` 只在启动时生效。本技能默认使用 `caterpiller_cad`；这是仓库现有 world 文件的
  拼写，文件为 `src/easygo_sim_bringup/worlds/caterpiller_cad.sdf`。所有启动命令显式传入该参数，
  不依赖脚本自身的默认 world。
- `script/full_sim.sh` 是人工交互入口，未提供关闭 GUI 的参数。AI/CI 做 headless 自测时，进入
  开发容器后按 `src/easygo_sim_bringup/README.md` 的「AI/CI 仿真自测」直接启动 launch，并显式设置
  `world:=caterpiller_cad use_gz_gui:=false use_rviz:=false`。
- 多车全量仿真资源消耗高。总内存为 24GB 或更低时不建议启动多车；默认选择单车仿真，或在
  有更高配置机器后再验证多车隔离行为。

## 验证决策

加载本技能不等于必须启动全量仿真。agent 应结合改动范围、风险、已有单测或 launch 测试、以及
交付所需证据，选择最小但足够的验证方式。

- 纯函数、配置静态检查、单节点逻辑或已有针对性测试足够时，不启动全量仿真，并在交付中说明。
- 需要确认跨组件链路、真实用户操作反馈、Gazebo 可见行为或多车隔离时，启动相应的单车或多车
  仿真；不要为了无关改动启动重栈。
- 改动共享 launch、端口、world、observe 或 webgz 时，必须评估单车和多车的影响。根据风险、机器
  资源和已有测试决定是否实际启动两种模式；未运行的一侧须在交付中说明原因及静态审查、launch 测试
  或其他替代证据。

## 验证范围

在修改需要真实跨组件行为或用户操作反馈的功能时使用本技能，例如 Gazebo world/模型/传感器、
TF 与 bridge、SLAM/定位、Nav2 导航与避障、遥控与运动控制、easygo-app/WebApp 业务流程、
VDA5050/MQTT，或多车的 DDS 域、端口、出生位姿和共享 Gazebo 兼容性。

交互验证包括 WebApp 的地图/定位/任务流程、驾驶台遥控与游戏模式、观察台和 RViz 的显示与控制、
语音助手页面的状态反馈，以及浏览器操作是否正确驱动后端、ROS 和仿真中的可见行为。

纯函数/单节点改动优先运行单测或 launch 测试；只需静态检查、构建或接口级验证时，不启动全量
仿真。涉及共享 launch、端口、world、observe 或 webgz 时，评估两种入口，并按「验证决策」保留
相应的运行或替代验证证据。

## 测试用例与选择

后续在本节登记全量仿真测试用例。每个用例写明覆盖功能、适用模式（单车/多车）、前置条件、执行
命令和判定结果，便于 agent 按改动范围检索。

- 不把全部全量仿真测试用例作为每次开发的固定回归；仅执行与本次功能、改动模块或共享链路相关的
  用例。
- 改动影响多个功能时，选择其对应用例的并集；改动共享基础设施时，同时选择受影响的单车和多车
  用例，并遵循「验证决策」处理资源限制。
- 未执行相关用例时，在交付中说明原因和已有替代证据；不得将不相关用例的通过结果作为功能验证。

<!-- 在此按以下格式新增用例：
### <用例名称>
- 覆盖：
- 模式：单车 / 多车
- 前置条件：
- 执行：
- 通过条件：
-->

## 单车

```bash
# 构建并启动（本技能默认 world）
bash script/full_sim.sh --world caterpiller_cad

# 使用已有 install/ 启动
bash script/full_sim.sh --start-only --world caterpiller_cad

# 仅构建
bash script/full_sim.sh --build-only

# 停止
bash script/full_sim.sh --stop
```

## 多车

先确认机器资源适合多车。24GB 内存机器不要用以下命令启动多车；多车验证不能由单车结果替代，
应安排到高配置机器执行。

```bash
# 构建并启动，默认 2 车（本技能默认 world）
bash script/full_sim.sh --multi --world caterpiller_cad

# 3 车指定场景与出生位姿
bash script/full_sim.sh --multi --robot-count 3 \
  --world caterpiller_cad \
  --poses '0.9,1.0,0;1.2,3.0,3.14;1.5,5.0,0'

# 仅构建 overlay，或复用 .sim-overlay/install 启动
bash script/full_sim.sh --multi --build-only
bash script/full_sim.sh --multi --start-only --robot-count 3 \
  --world caterpiller_cad

# 停止；需要删除容器时才执行 rm
bash script/full_sim.sh --multi --stop
bash script/sim-launcher/easygo-sim rm --no-update
```

启动前检查 `11345`、`9090`、`1883`、`8080`、`18088` 等冲突端口。多实例必须设置端口和
容器前缀偏移，具体参数以 `src/easygo_sim_bringup/README.md` 为准。

## 就绪与排错

- 多车启动器持续显示 `still starting` 时，先运行 `docker ps -a`，再检查各车前端、观察台、
  WebGZ 和 RViz 的 HTTP 端口。不要只根据容器处于 `Up` 判断成功。
- 多车默认端口：robot1 为前端 `18188`、观察台 `8190`、驾驶台 `6180`、RViz `6181`；后续
  robot 的 HTTP 端口按 100 递增。
- 检查某台车：`docker logs --tail 100 easygo-multi-robot1`。检查 RViz：
  `curl -fsS http://127.0.0.1:6181/vnc.html >/dev/null`。
- overlay 构建出现 `/ws` 与开发容器工作区的 CMakeCache 路径冲突时，重新运行
  `bash script/full_sim.sh --multi --build-only`。脚本会清理受影响包的构建缓存并保留 install。
- 仿真未完整退出时按 README 使用 `sim-kill.sh` 或多车 `rm`；不要用全局 `pkill`。
