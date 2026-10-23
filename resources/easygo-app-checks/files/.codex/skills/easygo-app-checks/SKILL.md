---
name: easygo-app-checks
description: Use when running easygo-app Rust builds, targeted tests, formatting, or precommit in EasyGo worktrees, including hmi music-manifest and ROS library environment failures.
---

# EasyGo App 检查入口

在现有 easygo-dev-main 容器检查指定 App 工作树。宿主不装 Rust/ROS，不修改其他容器或全局环境。

使用本 skill 的脚本传递原始命令参数，避免手拼 ROS、接口 SDK、音乐资源和测试 domain：

```bash
python3 /home/standard/.codex/skills/easygo-app-checks/scripts/run.py APP_WORKTREE -- cargo nextest run -p easygo-app -E 'test(TEST_NAME)'
python3 /home/standard/.codex/skills/easygo-app-checks/scripts/run.py APP_WORKTREE -- just precommit
python3 /home/standard/.codex/skills/easygo-app-checks/scripts/run.py APP_WORKTREE -- cargo build -p easygo-app --profile deploy
```

脚本从 App 主检出的相邻 easygo-music-resources 找 manifest.system.yaml，使用已有环境变量 EASYGO_MUSIC_MANIFEST 时尊重该显式路径；缺文件直接失败。不要用容器 /sros/music 的旧默认清单迁就构建错误，也不要为定向测试删 HMI 声音映射。测试使用独立 domain 191 和单个 nextest worker。

定向测试先确认运行到了目标断言；shell、依赖、资源缺失不是产品 RED。有效反例和相关回归通过后再跑完整门禁，最后冻结源码与二进制。仿真运行期间不同时修改被测源码或构建；容器内检查通过不代表实际双车、远端 CI 或交付已通过。

本入口只处理检查环境，不启动/停止仿真，不部署，不授予提交、推送或消息发送权限。
