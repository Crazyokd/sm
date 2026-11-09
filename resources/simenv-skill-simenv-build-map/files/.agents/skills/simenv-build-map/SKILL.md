---
name: simenv-build-map
description: 给一个已存在的世界（worlds/<world>/gazebo/world.sdf）建出对应的车端地图，并产出可用的世界 bundle（地图档案 + manifest：坐标偏移 / 站点 / 初始位姿）。当用户说"给这个世界建个图""新加的世界还没有地图""换世界要重开图""bundle 里地图对不上"时使用。
---

# 给一个世界开图（产出 bundle）

顺序是硬的：**先确认世界能开，再开图；开图必须开过去；开完必须量偏移；最后用 `simenv start` 的收敛断言收口。**

## 0. 前置（不满足先修世界，别急着开图）

1. 世界是 bundle 目录：`deploy/assets/worlds/<world>/`，SDF 在 `gazebo/world.sdf`；改完必须
   `docker build -f deploy/Dockerfile -t simenv-world-gazebo:local .`（世界进程读镜像里这份；
   车端校验世界名靠 easygo Port 把宿主这份 SDF 只读挂进 app 容器）。
2. **连通性自检**：`python3 ~/.cache/simenv-scenarios/connectivity.py deploy/assets/worlds/<world>/gazebo/world.sdf`
   —— 该走的路径必须真的唯一。旧 corridor 的教训：车道墙立在开阔场地里，车从墙外就绕过去了
   （真值 y≈−1.7，而车道南墙只覆盖 y∈[−0.8,−0.6]）。最窄通道要 ≥ 车宽 + 余量
   （0.82 m 车 / 1.2 m 车道可用；**台阶会把 1.2 m 卡成 0.9 m**，横向错位要用缓变段而不是台阶）。
3. `initial_poses`（manifest 里，世界系 m/rad）决定 spawn（世界侧）与定位（车端），给就要给全，
   缺一台 `simenv start` 直接失败；与 `--poses` 同给也直接失败（两份真相不允许）。

## 1. 起一个"没有图"的栈

把 `deploy/assets/worlds/<world>/` **整个目录临时移出**（**裸世界合法**：只有"世界就绪"，不灌图、
不注站点、不放车；注意目录里躺着资产却没有 `bundle.json` 会**报错**，所以要移就移整个目录），
`go build -o simenv ./cmd/simenv` 后起栈（`--world/--env-image/--app-image` 必填）。
否则旧图与新世界不匹配 → 定位断言失败 → 整次 start 回滚，栈活不到开图。

- 车端日志级别要放开，否则开图/定位的 INFO 全被吞（镜像默认 `RUST_LOG=warn`）：
  `docker exec -u 0 <容器> sed -i 's/^RUST_LOG=.*/RUST_LOG=info/' /opt/sros/share/easygo_sim_bringup/config/easygo_app.env` + 重启容器。
- 同名图的旧注册会让 `mapping/start` 返回 `912004 Map name already exists`；**只删 `/sros/map` 里的文件不够**
  （注册表在宿主机 `~/.cache/simenv/robotN`），要 `docker rm -f` + `rm -rf ~/.cache/simenv/robotN` 再起栈。

## 2. 开图 + 驾驶

```
POST /api/v6/mapping/start   {"name":"<map_name>","mode":"create","type":0}
POST /api/v6/vehicle/teleop/enable {"enabled":true}
POST /api/v6/vehicle/teleop/cmd    {"linear":…,"angular":…}
```

- **必须开过去**：`gz set_pose` 传送对车端不可见（位姿靠里程计 + 扫描匹配），传送会把图开糊。
- 反馈用**仿真真值**（`docker exec simenv-world gz model -m robotN -p`，服务慢，超时给 25~30 s；
  或容器内 `/sim/robot/pose`）。不要用开图期间车端上报的位姿——那时它没定位。
- 直道用"航向 + 横向纠偏"：`目标航向 = 路径轴向 + atan2(k·(目标横向 − 真值横向), |v|)`，
  **倒车时横向项反号**；`angular = k2·(目标航向 − 真值航向)`，限幅。
- 覆盖：每个待避区/口袋都要进到，最好两遍；**最后必须开回建图起点附近**，否则
  `mapping/stop` 返回 `913005 Robot is too far from the mapping start point`。

## 3. 收图（**四步，缺一不可**）

1. `POST /api/v6/mapping/stop`（要 200，409 重试）→ 轮询 `GET /api/v6/mapping/status` 到 `phase=idle`。
2. **生成导航资源**：`POST /api/v6/map/<name>/nav`
   —— 有 `<name>.traj`（建图轨迹）时由轨迹 + 图**自动生成** `<name>.route.geojson`（路线图）
   并重算 `<name>.nav.pgm`；缺轨迹则只重算 nav.pgm、沿用旧的 route.geojson。
   **响应直接告诉你拓扑对不对**，别只看 HTTP 200：
   ```bash
   curl -s -X POST http://127.0.0.1:18189/api/v6/map/<name>/nav
   # → {"data":{"filename":"corridor.nav.pgm","topology_node_count":4,"topology_edge_count":3}}
   #   node/edge 为 0 = 拓扑没生成出来 → 图里仍然没有 route.geojson（等价于没跑这一步）
   ```
   **这一步以前一直漏跑**，后果是：地图只有 SLAM 侧的 `*.pgm/*.png/.lom/.info/.traj/.json`，
   没有 `nav.pgm`、也没有 `route.geojson`。
   （注意：起栈时车端**会自己重算 `nav.pgm`**，所以"有 nav.pgm"不代表跑过这一步；**认 `route.geojson`**。）
3. **校验资源齐不齐**（缺任何一个都别往下走）：
   ```bash
   docker exec <容器> bash -lc 'ls -la /sros/map/ | grep <name>'
   # 必须看到：<name>0.pgm(+1/2…) <name>_*.png <name>.lom <name>.info
   #          <name>.json  <name>.traj  <name>.nav.pgm  **<name>.route.geojson**
   ```
4. 导出：`GET /api/v6/map/<name>/export` 常 404，直接 `docker cp <容器>:/sros/map` 取文件，
   打包成 `<world>.map_export`（`.tar.gz`）放进 `deploy/assets/worlds/<world>/easygo/`。

> **为什么 nav.pgm / route.geojson 是硬要求**：车端 Nav2 的 `compute_route`（路线图路由）拿不到
> `<map>.route.geojson` 就直接 `error_code:402 No route graph is available` → 任务被取消 → **车一步不动**。
> sim 侧 `easygo_sim_bringup/route_graph.py` 只做一件事：找 `<map_dir>/<map_name>.route.geojson`。

## 4. 量偏移（world − map）——只在有特征处量

`python3 ~/.cache/simenv-scenarios/measure_offset.py`：把车摆在**已知世界位姿**（读真值确认）→
`POST /api/v6/vehicle/localization/initial-pose {map_id,x,y,theta,loctype:2}`（mm / mrad）→
读回上报的**地图**位姿 → `offset = 世界 − 上报`。

- **只在有特征处**（两端待避区这种）：直线车道里 x 弱可观测 + 传送打乱 odom，读数会在
  −1.4 m ~ +1.2 m 之间乱跳（实测）。图内拟合（`fit_offset.py`）只作交叉检查，y 方向约束很弱。
- **多点必须一致**：真正的偏移是刚性量（实测两端相隔 28 m 只差 17 mm）。不一致 = 图或世界有问题，
  先查清楚再写 manifest。
- 顺手用 `verify_map.py` 看图的自由带与 SDF 墙面对不对得上。

## 5. 写 manifest

`deploy/assets/worlds/<world>/bundle.json`（严格解析，未知字段直接失败）：

```json
{
  "world": "<world>",
  "stations": [{"id": 1, "x": 1.0, "y": 2.0, "yaw": 0.0}],
  "initial_poses": [{"index": 1, "x": 0.0, "y": 0.0, "z": 0.05, "yaw": 0.0}],
  "source_sha": "05316d350aadc40f2d1e7bbbe2ed09d4a1092eb1",
  "worlds": {"gazebo": {"sdf": "gazebo/world.sdf"}},
  "vehicles": {"easygo": {"map_export": "easygo/<world>.map_export",
                          "map_name": "<map_name>",
                          "origin_offset_mm": {"x": 0.0, "y": 0.0}}}
}
```

- **`stations` 是世界系 m/rad**（不再写地图系 mm；换算 `map_mm = world_m*1000 − offset_mm`
  是 easygo Port 自己的事，仓内脚本从旧地图系数据换算过一次，不要手抄）。
- 地图只进 `vehicles.easygo` 节（`map_export` 相对世界目录）；`source_sha` 可选，有就记地图/世界来源修订。
- scenarios 不进 bundle（场景组归 harness runner，后续收仓单开）。

## 6. 收口（三条都过才算完）

1. `go test ./...` —— 仓内已有校验：manifest × 资产一致（各 Port 的节与其引用的文件成对出现）、
   **声明坐标必须落在地图观测过的区域**（把站点临时挪到未观测区应当当场红，可用来自测判定力）；
2. `./simenv start --robots 2 --world <world> --env-image simenv-world-gazebo:local --app-image …`
   **退出 0**，且两车上报位姿与 bundle 声明的**地图位姿**差在容差内（实测 corridor 5~8 mm、arena 8~15 mm）；
3. **车真的会动**：起栈后给一台车发个到站任务（`POST /api/v6/tasks` + `current/start`），确认它
   **真的开起来**（不是"任务取消/一步不动"）。只验 start 退出 0 是不够的——图缺 `route.geojson`
   时起栈照样成功，车却一步不动。

## 踩过的坑（真踩，按重要性）

| 现象 | 真因 |
|---|---|
| 起栈车端 API 长时间无响应（**报错在 stderr**，stdout 日志是空的） | 资产目录找不到世界文件 → 世界 SDF 挂接静默跳过 → 车端校验世界名失败。已修：资产根相对可执行文件解析（`assets.Path`），挂接源强制绝对路径；另注意 app 冷启动 1~3 分钟（就绪窗口 5 min） |
| 定位偏 2.2 m | 世界进程把出生位姿 yaw 丢了（create 请求写死朝向）+ 朝向比较没折角。已修 |
| 摆位判"定位未收敛"但车其实站对了 | 判据不能看 conf（conf = 匹配分数×100，corridor 长期 0~80 波动）；**判位置**（≤150 mm） |
| 摆位连续失败 | 车端还没挂上地图（指纹：`map_id` 为空）→ 先等它报出地图名（冷启动 2~4 min） |
| 图糊 / 匹配分数低 | 开图时用了传送；或没开回起点就 stop |
| 站点/位姿落在图外 | 图的覆盖没到那儿；站点必须在观测过的区域 |
| **车一步不动 / 任务被取消**，日志 `compute_route ... error_code:402 'No route graph is available'`，且 app 刷 `feasible_area_guard: failed to load feasible area JSON` | 收图漏了 **`POST /api/v6/map/<name>/nav`** → 图里没有 `route.geojson`（也没有 `nav.pgm`）。见 §3 第 2 步 |

## 相关

工具都在 harness 目录（仓库外 `~/.cache/simenv-scenarios/`）：`connectivity.py`（连通性）、
`measure_offset.py`（量偏移）、`fit_offset.py` / `verify_map.py` / `check.py` / `fit.py`（图↔世界校核）、
开图驾驶脚本、`lib.sh`（起栈）。
合同：`docs/superpowers/specs/2026-11-05-ports-architecture.md`（§5 bundle 格式、§8 配置与 flag）。
