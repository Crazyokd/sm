---
name: sim-deploy
description: Use when managing the SROS simulation environment, especially on a remote host, including start, update, status, diagnostics, logs, stop, down, or reset operations.
argument-hint: "[start|update|status|diag|logs|stop|down|reset] [ssh-host]"
---

# 仿真环境部署

管理 SROS 仿真环境，**默认以远程主机为主场景**。  
如果用户没有明确说“本地”，优先考虑 `remote` 子命令；本地命令是补充路径。

## 主路径

远程主机是主要使用方式，典型命令：

```bash
cd simulation
./scripts/deploy.sh remote start <host>                   # 默认实例 main
./scripts/deploy.sh remote start <host> --instance=<name> # 指定实例（可并行多个）
./scripts/deploy.sh remote start <host> --instance=<name> --state-mode=persistent
./scripts/deploy.sh remote build-update <host> --instance=<name>
./scripts/deploy.sh remote status <host>                  # 聚合所有实例
./scripts/deploy.sh remote diag <host>
./scripts/deploy.sh remote logs <host> [service]
./scripts/deploy.sh remote stop <host>
./scripts/deploy.sh remote down <host>
./scripts/deploy.sh remote down <host> --instance=<name>  # 删实例目录释放 slot，保留 persistent 状态
./scripts/deploy.sh remote down <host> --instance=<name> --purge-state
./scripts/deploy.sh remote reset <host>
```

常见主机示例：

```bash
./scripts/deploy.sh remote status dev-gpu-5060
```

## 次路径

只有在用户明确要求本地，或远程主机不可用时，才使用本地命令：

```bash
cd simulation
./scripts/deploy.sh start
./scripts/deploy.sh update
./scripts/deploy.sh status
./scripts/deploy.sh diag
./scripts/deploy.sh logs [service]
./scripts/deploy.sh stop
./scripts/deploy.sh down
./scripts/deploy.sh reset
```

## 命令说明

| 命令 | 远程主路径 | 本地补充 | 说明 |
|------|-----------|---------|------|
| `start` | `remote start <host>` | `start` | 首次部署或镜像重建；persistent 前自动备份 |
| `build-update` | `remote build-update <host>` | - | 编译并更新 runtime；persistent 前自动备份 |
| `status` | `remote status <host>` | `status` | 查看容器状态 |
| `diag` | `remote diag <host>` | `diag` | 看进程、端口、Gazebo 响应 |
| `logs` | `remote logs <host> [service]` | `logs [service]` | 跟踪日志 |
| `stop` | `remote stop <host>` | `stop` | 停止容器 |
| `down` | `remote down <host>` | `down` | 删容器和实例发布目录；persistent 状态默认保留 |
| `reset` | `remote reset <host>` | `reset` | clean 恢复基线；persistent 备份后清空选择性状态 |

## 状态模式

- `clean`：默认模式，供 CI/自动化使用；每次 runtime 重建都恢复 baseline DB 和地图并重新应用 profile。
- `persistent`：长期手工调试使用。首次执行 `remote start <host> --instance=<name> --state-mode=persistent` 启用，之后 `start/build-update` 不必重复传参。
- persistent 只保存 DB、地图、机器人图标配置，不保存整个 `/sros`；二进制、Web 和运行库仍来自当前 runtime image。
- 每次 `start/build-update` 前自动备份，固定保留最近 2 份，目录为 `~/sros-sim/persistent/<instance>/backups/`。
- `remote down` 保留 persistent 状态；只有 `--purge-state` 才彻底删除。`remote reset` 会先备份再清空选择性状态。
- `remote sim-test` 只能使用 clean 实例，避免长期手工状态污染自动化结论。
- `remote status` 输出实例的 `mode=clean|persistent`。

## 前置条件

- 必须在 `simulation/` 目录或项目根目录下执行
- 远程场景需要目标主机已完成基础初始化，并可 SSH 访问
- 本地场景需要 Docker / docker compose 可用，且 `.env` 配置完整

## 选择规则

- 用户提到具体主机名、`remote`、远程仿真机、远程服务器时：使用远程命令
- 用户只说“看仿真状态 / 部署仿真 / 更新仿真”，且上下文已有最近使用的远程主机：优先沿用该远程主机
- 用户明确说“本地”时：使用本地命令

## 远程主机继承

当用户没有显式给出 `<host>` 时，按下面规则处理：

1. 当前会话里最近一次成功使用过远程仿真主机时，默认沿用该主机
2. 如果上下文里只有一个明确的候选主机，例如 `dev-gpu-5060`，默认使用它
3. 如果同时存在多个候选主机，且无法判断用户指的是哪一台，先简短追问，不要猜
4. 只有在没有远程主机上下文，且用户明确说“本地”或场景明显是本地时，才走本地命令

示例：

```text
前文刚执行过：./scripts/deploy.sh remote status dev-gpu-5060
用户随后说：$sim-deploy diag
=> 默认理解为：./scripts/deploy.sh remote diag dev-gpu-5060
```

## 容器架构

远程多实例（端口 slot 化，`200<slot>xx`）：

```
sros-<inst>-gz-sim (vnc:200<slot>30, novnc:200<slot>80) ──grpc:200<slot>60──>
  sros-<inst>-sros-sim-runtime (webui:200<slot>10, proto:200<slot>01) ──> sros-<inst>-rerun-viewer (200<slot>90)
（main=slot0：20030/20080/20060/20010/20001/20090）
```

本地单实例（经典端口，不变）：

```
sros-gz-sim (:5901, :6080)  ──gRPC:50060──>  sros-sim-runtime (:8080, :5001)  ──gRPC:9876──>  sros-rerun-viewer (:9090)
```

## 注意事项

- runtime 更新使用 `remote build-update`；`remote update` 已废弃
- 改了 `simulation/docker/gz-sim/` 或 `simulation/bridge/` 后，先在远端代码目录用 `./scripts/build_images.sh --only=gz-sim` 构建推送新镜像，再用 `GZ_SIM_IMAGE_TAG=<new-tag> ./scripts/deploy.sh remote start <host>` 生成新的 release lock。`remote sim-test` 只读取 `~/sros-sim/instances/<name>/current` 下的 digest lock，不会从 `.env` 重写远端运行态。
- 验证 dev-gpu 主机是否真正走 GPU 渲染时，不能只看容器 runtime 或启动日志。还要确认 `docker exec sros-<name>-gz-sim grep -R "GZ_SERVER_DISPLAY\\|DISPLAY=" /etc/supervisor /app/entrypoint.sh` 中 `gazebo-server` 使用 `GZ_SERVER_DISPLAY`，并用 `nvidia-smi pmon -c 1` 看到 Gazebo/world 进程产生 GPU 负载。
- `reset` 会清空数据库、地图和机器人图标；persistent 模式会先留下最近备份
- 远程 `status` / `diag` / `logs` 通常是最常用入口，优先用来确认环境是否健康
- 如果本轮对话已经锁定远程主机，后续 `status` / `diag` / `logs` / `update` 默认继承该主机
