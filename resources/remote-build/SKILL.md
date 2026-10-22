---
name: remote-build
description: Use when building, syncing, deploying, or rebuilding SROS on a remote host, especially for build-update, remote start, or cross-host deploy workflows.
argument-hint: "[build-update|start|sync|build|deploy|build-clean|build-list|conan-install] [ssh-host]"
---

# 远程编译 + 部署

在远程服务器上编译并部署 SROS，**默认以远程主机为主场景**。  
如果当前会话已经锁定仿真主机，优先继承该主机，而不是每次都要求用户重复输入。

## 主路径

### 1. 最常用：同步 + 编译 + 更新仿真

```bash
cd simulation
./scripts/deploy.sh remote build-update <host>
```

这是最常用路径：同步源码、远程编译、同步二进制、更新仿真运行环境。

示例：

```bash
./scripts/deploy.sh remote build-update dev-gpu-5060
./scripts/deploy.sh remote build-update dev-gpu-5060 --preset=amd64-release
./scripts/deploy.sh remote build-update dev-gpu-5060 --name=feature-x
```

### 2. 全量重建：同步 + 镜像重建 + 启动

```bash
cd simulation
./scripts/deploy.sh remote start <host>
```

适用场景：

- 改了 `gz-sim-bridge`
- 改了 Dockerfile / entrypoint
- 需要从干净环境重新拉起仿真

### 3. 其他远程编译操作

```bash
# 仅同步源码
./scripts/deploy.sh remote sync <host>

# 仅编译（不部署到仿真）
./scripts/deploy.sh remote build <host>

# 跨机部署（编译机 -> 仿真机）
./scripts/deploy.sh remote deploy <build-host> <sim-host>

# conan 依赖重装
./scripts/deploy.sh remote conan-install <host>

# 清理工作区
./scripts/deploy.sh remote build-clean <host>

# 列出工作区
./scripts/deploy.sh remote build-list <host>
```

## 与其他技能的衔接

编译完成后，最常见的后续动作是：

```bash
# 看远程仿真状态
./scripts/deploy.sh remote status <host>

# 做远程诊断
./scripts/deploy.sh remote diag <host>

# 跑远程分级测试
./scripts/deploy.sh remote sim-test <host> smoke
```

也就是说，`remote-build`、`sim-deploy`、`sim-test` 默认应该沿用同一台远程主机。

## 前置条件

- 远程主机已配置 SSH 免密登录
- 远程主机已安装 Docker
- 首次使用需运行 `./scripts/deploy.sh remote setup <host>`
- 远程机器需要具备编译所需磁盘空间和缓存卷

## 编译隔离机制

- 远程编译工作区位于 `~/sros-build/<branch-name>/`
- 仿真运行目录位于 `~/sros-sim/`
- 所有编译工作区共享 Docker volumes：`sros-conan-cache`、`sros-ccache`，其中 ccache 统一挂载到容器内 `/ccache`
- 可用 `--name=xxx` 覆盖默认分支名

## 选择规则

- 用户说“远程编译 / 编译更新 / build-update / 远程部署”时：优先 `remote build-update`
- 用户说“桥接代码改了 / entrypoint 改了 / 镜像要重建”时：优先 `remote start`
- 用户说“只同步 / 只编译 / 列出工作区 / 清理工作区”时：走对应远程编译子命令
- 用户说“跨机部署”时：使用 `remote deploy <build-host> <sim-host>`

## 远程主机继承

当用户没有显式给出 `<host>` 时，按下面规则处理：

1. 当前会话最近一次成功使用过远程仿真主机或编译主机时，默认沿用该主机
2. 如果上下文中只有一个明确候选主机，例如 `dev-gpu-5060`，默认使用它
3. 如果存在多个候选主机且无法判断，先简短追问，不要猜
4. `remote deploy` 是例外：它需要两个主机，不能自动猜第二个主机

示例：

```text
前文刚执行过：./scripts/deploy.sh remote status dev-gpu-5060
用户随后说：$remote-build build-update
=> 默认理解为：./scripts/deploy.sh remote build-update dev-gpu-5060
```

## 注意事项

- `build-update` 只更新 SROS 二进制；如果改了 gz-sim-bridge / Dockerfile，需用 `remote start`
- `remote reset` 会清空 DB 卷，通常属于 `sim-deploy` 范畴，不属于编译主路径
- entrypoint 的 `apply_model_config()` 会覆盖手动 DB 修改
- 如果本轮对话已经锁定远程主机，后续 `build-update` / `start` / `status` / `sim-test` 默认继承该主机
