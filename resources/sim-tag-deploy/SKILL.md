---
name: sim-tag-deploy
description: Fast-deploy SROS simulation from Git tag(s) to remote simulation instances, usually on std@10.10.21.75, by checking out tags in isolated worktrees, syncing tagged simulation inputs, pulling prebuilt Nexus images, and running simulation/scripts/deploy.sh remote start. Use when the user asks to deploy a tagged SROS simulation version, switch simulation versions, run one or two simulation instances, compare two different tags on separate instances, roll back to an older tag, or prepare tagged simulation environments for testing.
---

# SROS Tag 仿真部署

## 默认语义

当用户说“部署某个 tag 的 SROS 仿真”且没有额外参数时，按下面默认值执行：

```text
host: std@10.10.21.75
instances: main
mode: fast
runtime image: nexus3.sr:8060/sros/sros-sim-runtime:<Git tag>
headless: true
```

这表示只部署远端主机上的默认实例 `main`，不执行远端 `build-update`。

远端仿真机必须使用 `std` 用户 SSH。不要把默认主机写成裸 IP `10.10.21.75`，否则 `ssh` 会使用当前电脑用户名，常见表现是 `Permission denied`。如果本机 SSH config 里有等价 alias（例如 `sim` -> `std@10.10.21.75`），也可以显式传 `--host sim`。旧主机 `10.10.16.124` 已退役，禁止用于新部署或验证。

只有当用户明确说“另一个实例”“两个实例”“main 和 sim2”“对比两个版本”，或显式给出 `--instances` / `--pairs` 时，才部署多个实例。多实例可以是相同版本，也可以是不同版本。每个实例有独立远端目录、端口、容器和 release lock。

如果用户表达含糊，例如“也部署一份”“再起一个环境”，但没有说明实例名或版本关系，先简短确认实例名和 tag，不要自行猜测。

## 主入口

优先使用随 skill 提供的脚本，避免手工 checkout 污染用户当前工作树：

```bash
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh <tag>
```

示例：

```bash
# 默认部署到 std@10.10.21.75 的 main，关闭 Gazebo GUI
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh v5.70.0

# 指定主机和两个实例
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh v5.70.0 --host std@10.10.21.75 --instances main,sim2

# 部署后对每个实例跑 smoke
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh v5.70.0 --smoke

# 两个实例运行不同版本
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh --pairs main=v5.70.0,sim2=v5.71.0

# 显式走旧的远端编译慢路径
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh v5.70.0 --build
```

## 工作流

默认 `fast` 模式下，脚本会做：

1. 在当前仓库中 `git fetch --tags`。
2. 在临时 worktree 中 checkout 指定 tag，不切换用户当前分支。
3. 从该 tag 的 `simulation/sim-images.lock.json` 读取 `GZ_SIM_IMAGE_TAG` 和 `RERUN_IMAGE_TAG`。
4. 将 `SROS_RUNTIME_IMAGE_TAG` 设为 Git tag 本身，例如 `v5.72.0`。
5. 调用 `simulation/scripts/deploy.sh remote start <host> --instance=<instance>`，由底层脚本同步 tag worktree 的 profile/world/map/cfg 等运行输入到远端实例目录，并从 Nexus 拉取三张镜像后重建 compose。
6. 默认用 `--headless` 启动；如果用户显式要求 `--gui`，启动后会等待 VNC/X server 可用并关闭 X screen saver，避免 noVNC 挂一段时间后黑屏。
7. 可选执行 `remote sim-test ... smoke`。

历史 tag 如果还没有 `simulation/sim-images.lock.json`，脚本会用当前仓库的 lock 读取 `gz-sim` / `rerun-viewer` tag，并把它们作为显式环境变量传给 `remote start`。这只影响镜像 tag 选择，不会切换或修改用户当前分支。

同版本多实例模式下，只 checkout 一份 tag worktree，然后对每个实例执行 `remote start`。

不同版本多实例模式下，使用 `--pairs`：

```bash
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh --pairs main=v5.70.0,sim2=v5.71.0
```

脚本会分别 checkout 每个 tag，并让每个实例使用各自 tag 对应的源码输入和 runtime 镜像 tag。

## 显式构建模式

只有当用户明确要求“现场构建”“没有预构建镜像”“重新 build-update”，或确认 Nexus 上缺少 `sros-sim-runtime:<Git tag>` 时，才使用：

```bash
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh v5.70.0 --build
```

`--build` 会恢复旧行为：

1. 在 tag worktree 的 `simulation/` 下执行 `remote build-update`。
2. 从构建后回填的 `sim-images.lock.json` 读取 `SROS_RUNTIME_IMAGE_TAG`。
3. 再执行 `remote start`。

这是慢路径，默认不要使用。

历史 tag 没有 `simulation/sim-images.lock.json` 时，`--build` 无法从 tag worktree 回读构建后的 runtime tag；这类场景优先使用默认 fast 模式，或在用户明确知道镜像 tag 时同时传 `--runtime-tag <tag>`。

## 版本切换

切换版本就是重新运行脚本并传另一个 tag：

```bash
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh v5.71.0
```

回滚同理：

```bash
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh v5.70.0
```

如果用户要求两个版本同时跑，用 `--pairs`：

```bash
sros-core/.agents/skills/sim-tag-deploy/scripts/deploy_sim_tag.sh --pairs v5_70_0=v5.70.0,v5_71_0=v5.71.0
```

## 注意事项

- CI tag pipeline 应发布 `nexus3.sr:8060/sros/sros-sim-runtime:<Git tag>`；skill 默认按这个 tag 拉取 runtime 镜像。
- `gz-sim` / `rerun-viewer` 通常复用 `simulation/sim-images.lock.json` 中记录的已有镜像；当 simulation 相关镜像输入变更时，由 CI 的条件 job 构建并推送新的 gz 镜像。
- 历史 tag 没有 `simulation/sim-images.lock.json` 时，默认 fast 模式会回退使用当前仓库 lock 的 `gz-sim` / `rerun-viewer` tag；新 tag 应始终提交 lock。
- 默认只部署 `main` 实例；不要在用户没明确要求时启动第二个实例。
- `remote start` 仍然需要从 tag worktree 同步 profile/world/map/cfg 等运行输入，因为 compose 会把这些目录只读挂进容器，它们不完全属于 runtime 镜像。
- 若目标主机首次使用或 Docker/GPU 环境未初始化，先执行 `cd simulation && ./scripts/deploy.sh remote setup <host>`。
- 如果默认 fast 模式在 pull 阶段失败，先确认 tag pipeline 是否已推送 runtime 镜像；不要直接改为 `--build`，除非用户确认接受慢路径。
