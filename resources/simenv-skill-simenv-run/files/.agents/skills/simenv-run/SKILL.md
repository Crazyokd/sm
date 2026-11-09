---
name: simenv-run
description: Start, status, or stop the independent simenv world plus N runtime-sim app containers. Use when bringing up the sim environment, checking GZ clock / robotN / API, or tearing it down.
---

# 跑独立仿真环境

本仓只起环境和编排。程序镜像由 `--app-image` 传入。不要在本仓起 easygo-app，不要包装 EasyGo 旧入口。

## 何时用

用户要 `start` / `status` / `stop` 仿真环境，或看 clock、`robotN`、API 是否接通。

## 闭环

**1. 构建。** 需要环境镜像或本机没有 `./simenv` 时：

```bash
go test ./...
go build -o simenv ./cmd/simenv
docker build -f deploy/Dockerfile -t simenv-world-gazebo:local .
```

**2. 启动。** `--world` / `--env-image` / `--app-image` 都必填，没有默认值：

```bash
./simenv start --robots 1 --world simple \
  --env-image simenv-world-gazebo:local \
  --app-image docker-registry.nowhere.sr/easygo/runtime-sim:<tag>
```

加 `--rviz` / `--gz-gui` 开观察视图（按车编号逗号分隔，如 `--rviz 1,3` 只给 robot1 和 robot3 开）：web-gz 3D 在 `http://127.0.0.1:6180/vnc.html`，RViz web 在 `http://127.0.0.1:6181/vnc.html`（端口按契约算术 `base+100N`）。默认 headless。

可选 `--session <name>` 指定会话名。不写就生成一个，例如 `brisk-otter`，打在 start 的第一行。`simenv status` 列出全部会话，`simenv status <name>` 看这一路。`simenv stop` 停最近启动的那一路，`simenv stop <name>` 停指定的那一路。每个会话的容器名、GZ partition 和车端端口按名字错开。

**gz-gui 一个就够，不要每车一个。** GUI 渲染的是整个世界，收到什么画什么——`--gz-gui 1` 的单窗口里所有车都在画面里，`WEBGZ_FOLLOW_TARGET` 只决定相机跟谁、不决定画谁。每多开一个 GUI 买到的只是一台额外的自动跟随镜头，代价是又一份全场景渲染（约 5.4 核 / 车）。RViz 则相反，它连的是各车独立的 ROS domain，只能看见自己那台车，所以要按车开。

N=1 的 API 是 `http://127.0.0.1:18189/api/v6`，不是 `:18089`。

**3. 验收。**

- `./simenv status`：world running、robots 含 `robot1`（非空即世界已就绪，时钟与实体是世界内部的等待条件）、vehicle `robot1` running、`connected=true`
- 环境容器进程没有 easygo-app / slam_mapping / nav2
- 程序在跑时 `GET http://127.0.0.1:18189/api/v6/vehicle/status` 可达
- 带 `--rviz` / `--gz-gui` 时对应车端口的 noVNC 页面返回 200（N 号车：rviz `6081+100N`、gz-gui `6080+100N`）
- N≥2 实时性：`gz topic -e -t /clock` 采样，仿真时间增量应等于墙钟增量（实时率 1.0）。宿主无 `/dev/dri` 时退软件渲染，N=2 只有 ~0.48
- 本机没 Gazebo / `runtime-sim` 时写明未验证。没跑过多车不是环境缺陷

**4. 停止。** `./simenv stop`。两边停干净，`18189` 释放。

## 禁止

- 用 `docker run` 拼世界或车端，绕过 `simenv`
- 环境镜像 `FROM runtime-sim`、运行时挂 EasyGo 源码
- 跑 `full_sim.sh` / `easygo-sim` / `sim_multi_gazebo.launch.py`
- 把 `:18089` 当 N=1 默认 API
