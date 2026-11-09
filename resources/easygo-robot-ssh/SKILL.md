---
name: easygo-robot-ssh
description: 为 EasyGo ARM/AMR 机器人配置和排查从当前开发机到机器人设备的免密 SSH 访问。适用于 Codex 需要配置 SSH Host alias、安装公钥、连接 easygo/300E/forlinx/linaro 等机器人、维护机器人目标清单或准备远程调试环境的场景。
---

# EasyGo 机器人 SSH

这个 skill 用来从当前开发机配置到 EasyGo 机器人 / ARM 设备的 SSH 访问，目标是让后续远程调试可以直接使用稳定的 SSH alias，而不是每次手写 IP、端口和用户。

## 基本原则

- 默认在当前开发机上执行 SSH 配置，除非用户明确指定其他机器。
- 不要把明文密码写进仓库。密码只允许通过交互输入，或通过当前命令临时使用 `EASYGO_SSH_PASSWORD`。
- 优先配置 SSH Host alias，后续调试命令统一使用 alias。
- 只更新目标 alias 对应的 `~/.ssh/config` block，不覆盖无关 SSH 配置。
- 如果目标机器不在清单里，先收集 alias、host/IP、user、port 和备注，再决定是否写入清单。

## 机器人清单

选择目标机器时读取 `references/robots.tsv`。当前内置 alias：

- `easygo`：第一台样机，`std@10.10.71.196:22`
- `easygo-300e`：300E 样车，`forlinx@10.10.68.213:2221`
- `easygo-300e-2`：300E 样车-2，`linaro@10.10.71.210:2222`

## 快速流程

1. 确认目标 alias；如果是新机器，先补齐 alias、IP、用户和端口。
2. 确认当前开发机上有 SSH key，通常是 `~/.ssh/id_ed25519.pub`。如果没有，先征求用户同意再生成。
3. 使用内置脚本配置免密登录：

```bash
# 在感知仓库根目录执行
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh setup easygo
```

4. 验证登录：

```bash
# 在感知仓库根目录执行
ssh easygo 'hostname; uname -a'
```

## 脚本用法

使用 `scripts/easygo_robot_ssh.sh` 执行稳定、可重复的 SSH 配置动作。

```bash
# 在感知仓库根目录执行
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh list
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh setup easygo
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh config easygo-300e
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh install-key easygo-300e-2
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh test easygo
```

密码传入方式按优先级选择：

```bash
# 推荐：交互输入密码
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh setup easygo

# 只在当前命令临时使用；不要写进仓库、文档或历史脚本
EASYGO_SSH_PASSWORD="$ROBOT_PASSWORD" bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh setup easygo
```

## 临时目标

如果机器人还没有写入清单，可以先用命令行指定一次性目标：

```bash
# 在感知仓库根目录执行
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh setup \
  --alias easygo-new \
  --host 10.10.x.x \
  --user forlinx \
  --port 22
```

目标稳定后，再把它加入 `references/robots.tsv`。

## 新增机器人

按 tab 分隔格式追加到 `references/robots.tsv`：

```text
alias	host	user	port	note
```

追加后运行：

```bash
# 在感知仓库根目录执行
bash .agents/skills/easygo-robot-ssh/scripts/easygo_robot_ssh.sh list
```

## 排查规则

- 如果缺少 `sshpass`，先说明需要在当前开发机安装或使用已有环境；不要未经用户同意直接安装软件包。
- 如果出现 `Permission denied`，先核对 `references/robots.tsv` 里的 user、port、host，再让用户确认密码。
- 如果出现 `REMOTE HOST IDENTIFICATION HAS CHANGED`，先停止并询问用户，不要直接改 `known_hosts`。
- 如果 SSH 已经通了，但机器人业务工具失败，切换到仓库里的调试或部署规则继续处理。
