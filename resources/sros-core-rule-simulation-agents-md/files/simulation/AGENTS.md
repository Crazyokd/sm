# simulation/AGENTS.md

本文件为在 `simulation/` 目录下工作的 Codex 提供导航。

> 完整仿真开发指南请阅读 [CLAUDE.md](./CLAUDE.md)，包含 gRPC 接口、数据库配置、pytest 合约、已知陷阱。

## 快速参考

### 部署命令

```bash
# 本地
./scripts/deploy.sh start          # 启动仿真环境（3 容器）
./scripts/deploy.sh update         # 热更新 SROS 二进制
./scripts/deploy.sh stop           # 停止仿真
./scripts/deploy.sh reset          # 重置数据

# 远程
./scripts/deploy.sh remote build-update <host>   # rsync + 编译 + 部署
./scripts/deploy.sh remote start <host>          # 全量重建
```

### 测试命令

新增或修改 `tests/` 下的测试、fixture 与测试 helper 前，必须阅读并遵循
[仿真测试开发规范](docs/test-development-guide.md)与
[仿真测试断言规则](tests/ASSERTION_RULES.md)。违反其中强制约束或命中断言规则的
“禁止模式”属于评审阻塞问题，测试当前为绿色不能作为例外理由。

```bash
# 在 simulation/ 目录下执行
pytest tests/live --sim-suite smoke -v    # 冒烟测试（MR 门禁，p0）
pytest tests/live --sim-suite core -v     # 核心合约（p0+p1）
pytest tests/live --sim-suite nightly -v  # 完整合约（p0+p1+p2）

# 回归主线（e2e）：gulf 取放货 + oasis 顶升搬运
pytest tests/live -m regression    # 或 -m "e2e and smoke"
# -m e2e 只选真 e2e 场景，不含单功能用例

# 单功能定位
pytest tests/live/functional/<域>       # 例如 tests/live/functional/navigation/
pytest tests/protobuf_test/<域>          # ONES 测试计划转换用例

# 静态秒级（无需仿真环境）
pytest tests/static

# 单个测试
pytest tests/live/functional/navigation/test_navigation.py::test_my_case -v
```

### 协议 CI 编排规范

- `scripts/protocol_ci.py` 与 `scripts/lib/protocol_prepare.py` 中的类和函数必须提供中文
  docstring，至少说明职责；涉及容器、网络、文件、环境变量或远程状态等副作用时，还应说明
  关键输入、执行顺序和失败行为。
- GitLab YAML 只声明触发条件、套件选择和产物；容器生命周期放在 `protocol_ci.py`，SROS/Gazebo
  业务状态准备放在 `protocol_prepare.py`，不得在两个协议 Job 中复制编排逻辑。

### 容器架构

```
sros-gz-sim (:5901, :6080)  →  sros-sim-runtime (:8080, :5001)  →  sros-rerun-viewer (:9090)
    Gazebo + VNC                    SROS + Web UI                      Rerun 可视化
         └── gRPC :50060 ──────────────┘          └── gRPC :9876 ──────────┘
```

## 注意事项

- 改 sros 代码/profile/world 用 dev 模式：`deploy.sh dev up` + `dev shell`，容器内 `simulation/scripts/dev_prepare_sros.sh` 生成 `/sros` 后跑 `build-amd64/bin/sros`。dev 挂宿主源码，**不过 drift 闸门**，不需要发布镜像；详见 [README.md](README.md)
- 修改 gz-sim-bridge 代码后需 `build_images.sh` 推新 tag + `--write-lock` 刷新并提交 `sim-images.lock.json`，再 `remote start` 重建镜像；`build-update` 只更新 SROS。未刷新 lock 部署会被 drift 闸门拒绝（`SIM_ALLOW_IMAGE_DRIFT=1` 跳过）
- 镜像 tag 不在 `.env` 配：本地与远程都从入库 `sim-images.lock.json` 取，`deploy.sh start`/`sim-test`/`world` 启动时自动注入；临时覆盖用 `GZ_SIM_IMAGE_TAG=<tag> ./scripts/deploy.sh start`
- Gazebo ContactSensor 插件顺序：`contact-system` 必须在 `sensors-system` 之前
- 测试前必须 `reset_robot_pose()` 重置位姿
- GCC 版本差异：SROS 用 GCC 9，bridge 用 GCC 13，注意 `std::atomic` 等差异
