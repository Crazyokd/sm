---
name: easygo-ble-monitor
description: Use when observing EasyGo Bluetooth peer facts, recording vehicle movement, checking reception against pose or distance, or needing a live terminal sidebar and offline evidence review (蓝牙、多车、挪车、实时监测、侧栏、0.2 Hz、断流、RSSI).
---

# EasyGo 蓝牙邻车监测

以一个选定双车对为观测对象，保留收到的其他邻车事实。把位姿、完整事实、连接状态和派生指标分开记录。默认只观测；用户负责挪车时，保持这个分工。

## 先选阶段

| 用户当前意图 | 执行范围 |
| --- | --- |
| “先准备，暂不读取” | 只准备本地工作目录和待机面板，车辆配置保持未填写 |
| “车辆好了，开始监测” | 核实本次目标，显式开始采集，验证两个连续刷新窗口 |
| “停止记录、汇总” | 用原会话目标停止本次记录器，确认结束，取回归档，再离线分析 |
| 分析已有记录 | 直接读本地会话；不连接车辆 |

沿用本次用户授权。监测本身不授予运动、定位、部署、重启、蓝牙开关或参数修改权限；本工具也不执行这些动作。

## 本地准备与显示

`SKILL_DIR` 为本 `SKILL.md` 所在目录。选择安装树外的本地可写目录 `BLE_RUN`；新建时使用空目录。已有工作目录保留配置和历史，使用其中的程序；升级程序时另建工作目录。

```bash
python3 "$SKILL_DIR/scripts/prepare.py" --workspace "$BLE_RUN"
python3 "$SKILL_DIR/scripts/panel.py" --workspace "$BLE_RUN" --pane-target "$MONITOR_PANE"
```

先确认 `MONITOR_PANE` 属于当前任务，优先使用已知 `TMUX_PANE`。面板保持聊天焦点，只复用带本工作目录标签的 pane。执行端无法确认 pane 时，在独立终端运行 `python3 "$BLE_RUN/sidebar.py"`；离线核对可用 `--snapshot`。

准备阶段不执行 SSH、车辆 HTTP、ROS 查询或启动采集。不要因昨天的 `active.json` 显示 recording 就把它当成本次会话。

本机需要 Python 3.10+、curses、OpenSSH；侧栏使用 tmux。采集时，车端需要现有 `easygo` 容器中的 Python/rclpy/std_msgs、运行中的 `peer-information`，以及主机的 dbus-python、btmon 和授权的无交互 sudo。缺少依赖时报告缺口，不自动安装或改服务。

## 明确开始后

1. 解析用户本次指定的目标。若使用 wx1/wx2 等 SSH 别名，先用本地 `ssh -G` 解析一次；随后以固定 IPv4、SSH 用户/端口、robot_id、蓝牙 MAC 填写工作目录中的 `targets.json`。wx1/wx2 只是显示标签，不再用别名轮询。
2. 用当前 API、唯一 bridge 进程和包元数据核实身份；不凭 hostname、旧地址或旧位姿推断。复用已有认证与已验证的 SSH 通道；不关闭主机密钥校验，不把密码或完整进程环境写进配置、Skill 或归档。
3. 执行 `python3 "$BLE_RUN/start_recording.py" --start`。程序固定目标及 SSH 通道、克隆 bridge 的 ROS 环境，并记录版本和启动时传输选择。UDP PeerLink 不能充当真实蓝牙证据。
4. 等待记录器就绪，再核对两个连续 5 秒窗口：self 计数增加、采样心跳新鲜、身份匹配、本地原始文件备份推进。peer 为 0 是一个测量结果，不应被当成采样程序成功或失败的唯一标准。

默认侧栏每 5 秒刷新，原始 ROS 事实按源频率接收，API 位姿约每 0.5 秒记录，链路/资源约每秒记录。先保存原始数据，再做摘要；无效消息保留原文，缺口标记，不填成 0。单会话采集上限两小时，启动时告知操作者。

面板位姿只展示同源且新鲜的定位；中心距离要求两车同图且已定位。低或未知置信度保留原值并标为估计；未定位、不同地图或过期则不作有效距离判断。

## 停止、归档、分析

```bash
python3 "$BLE_RUN/stop_recording.py" --stop
```

只停止本次会话的记录器与子进程，按固定目标检查身份；不停止车辆业务进程。停止请求不等于停止确认。无法连接、身份不符或存在无法归属的数据时保留 `cleanup_pending`，不要伪报待机或叠加新会话；确认根本未启动的组件标为未启动。

结束后取回完整 source/radio 归档和 SHA256；本地镜像可能落后，确认完整归档已取回后才宣布数据已归档。程序、配置和记录都留在工作目录，不写入安装的 Skill。

```bash
BLE_SESSION="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["local_directory"])' "$BLE_RUN/active.json")"
python3 "$SKILL_DIR/scripts/analyze.py" "$BLE_SESSION"
```

输出 `analysis/summary.json`、`report.md`、`timeline.csv`、`distance-bins.csv`。完整归档优先于部分本地镜像；缺失、损坏、身份不符、计数不一致或采集子进程退出均显式报告。可在记录中途离线分析当前本地镜像，但要标明仍是部分记录。

解释前读取 [数据与证据边界](references/data-and-evidence.md)。尤其注意：

- `Connected=true`、RSSI 或注册信息不代表完整事实已交付；两个方向分别统计。
- 接收点是 bridge 的 peer ROS 话题，不自动等于 Nav2 采纳或避障成功。
- 帧龄包含时钟差、采样发布与收集延迟；未校时的带符号数值必须标为 RAW，不能称单向传输时延。
- 远处偶尔收到、近处断流、未碰撞都不足以证明稳定最大距离或避障提升。

## 验证包本身

离线运行，不接触车辆：

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s "$SKILL_DIR/tests" -v
```

源码中的脚本通过解释器运行；不要依赖安装后可执行位，也不要把当前车辆地址、采样数据或主机信任状态提交到 Skill 源仓库。
