---
name: easygo-app-checks
description: Use when running easygo-app Rust builds, targeted tests, formatting, or precommit in EasyGo worktrees, including hmi music-manifest and ROS library environment failures.
---

# EasyGo App 检查入口

在现有 easygo-dev-main 容器检查指定 App 工作树。宿主不装 Rust/ROS，不修改其他容器或全局环境。

使用本 skill 的脚本传递原始命令参数，避免手拼 ROS、接口 SDK、音乐资源和测试 domain：

以下路径相对于本 Skill 目录。若当前在 Git 工作树中，先从该仓库主检出的 `.agents/skills/easygo-app-checks/` 读取本 Skill，再以实际工作树路径作为 `APP_WORKTREE`。

```bash
python3 scripts/run.py APP_WORKTREE -- cargo nextest run -p easygo-app -E 'test(TEST_NAME)'
python3 scripts/run.py APP_WORKTREE -- just precommit
python3 scripts/run.py APP_WORKTREE -- cargo build -p easygo-app --profile deploy
```

脚本从 App 主检出的相邻 easygo-music-resources 找 manifest.system.yaml，使用已有环境变量 EASYGO_MUSIC_MANIFEST 时尊重该显式路径；缺文件直接失败。不要用容器 /sros/music 的旧默认清单迁就构建错误，也不要为定向测试删 HMI 声音映射。测试使用独立 domain 191 和单个 nextest worker。

构建使用已解包参数包或测试使用独立参数进程时，可在调用环境中显式设置 EASYGO_PARAM_DEFINITIONS_DIR、EASYGO_PARAM_MANAGER_BIN、EASYGO_PARAM_MANAGER_CFG_DIR。脚本将已设置的这三项按原值传入容器；路径必须在容器内可访问。未设置的变量保留容器既有配置和目标构建/测试的默认行为。入口不自动下载或安装参数包，也不从相邻参数服务源码补齐公开定义。

定向测试先确认运行到了目标断言；shell、依赖、资源缺失不是产品 RED。有效反例和相关回归通过后再跑完整门禁，最后冻结源码与二进制。仿真运行期间不同时修改被测源码或构建；容器内检查通过不代表实际双车、远端 CI 或交付已通过。

本入口只处理检查环境，不启动/停止仿真，不部署，不授予提交、推送或消息发送权限。

可用 `EASYGO_ROS_INTERFACES_DIR` 显式选择已安装接口 SDK（目录内含 `install/setup.bash`）；必须与目标源码的接口版本匹配，接口变更后重新生成，不自动回退其他检出的 SDK。`CARGO_TARGET_DIR`、`ROS_LOG_DIR` 保留显式值，未设置时分别使用目标工作树的 `target/` 和 `.cache/ros-logs/`。目标目录必须是 App Git 根；宿主和容器内的必需路径均需可读，缺挂载时报告实际路径并停止，不能切换另一检出代替验证。

记录源码路径、HEAD、工作区差异、实际命令和真实退出码。用 `tail` 等筛选输出时使用 `pipefail` 或保存原命令状态；增加过修补的源码副本单独报告，不能算原提交通过。若当前 `justfile` 的 `precommit` 已包含 test 与 lint，同一源码状态不再先跑一次相同 lint；源码或环境变化后按影响重新验证。
