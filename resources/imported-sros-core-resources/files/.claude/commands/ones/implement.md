# ONES 需求实现

从 ONES 需求工作项出发，完成：**需求理解 → 技术方案 → 代码实现 → 测试 → MR 提交**。

## 参数

$ARGUMENTS

- 第一个参数：ONES 工作项 ID（序号）
- `--skip-plan`：跳过方案设计阶段（已有技术方案时）
- `--no-upload`：不上传实现报告到 ONES
- `--branch <name>`：指定在已有分支上工作（默认基于 main 新建）

---

## Phase 0：前置检查

### 0.1 获取需求详情与状态校验

按 `common.md` 认证，获取工作项详情。

**状态门禁**：检查 `status_uuid` 是否为 `5WJAdd7o`（需求导入）。
- 是 → 继续
- 否 → 输出当前状态名称，提示"该需求当前状态为「XXX」，非「需求导入」状态，无法开始实现。"并**停止执行**

### 0.2 变更状态为「开发中」

通过 transit API 将状态流转到「开发中」（`K7mLx1ej`）：
```
GET  /team/UNrQ5Ny5/task/<task_uuid>/transitions  → 找 end_status_uuid == K7mLx1ej 的 transition
POST /team/UNrQ5Ny5/task/<task_uuid>/transit       → {"transition_uuid": "<matched>"}
```

> transit 完成后**必须立即**用 `tasks/update` 把 `assign` 回写为 ONES token 用户 UUID（详见 common.md「默认负责人 / 问题解决人」）。否则 transit 副作用可能把解决人换成他人，统计无法归属当前帐号。

---

## Phase 1：需求理解

### 1.1 提取关键信息

从工作项中提取（详情已在 Phase 0 获取）：

| 维度 | 来源 |
|------|------|
| 功能描述 | summary + desc_rich |
| 验收标准 | 描述中的验收条件、预期行为 |
| 关联模块 | 描述中提到的模块名、文件名、配置项 |
| 约束条件 | 兼容性要求、性能要求、平台限制 |
| 关联工作项 | 相关缺陷或前置需求 |
| 已有分析报告 | 描述中嵌入的 AI 分析报告（根因、代码路径、修复建议） |

> **重要**：如果描述中包含 AI 分析报告（如 `ones-analyze` 生成），直接提取其中的根因、因果链、关键代码路径和修复建议，作为实现方案的输入，避免重复分析。

### 1.2 变更类型判断

根据需求内容判断变更类型，决定后续 Phase 的执行路径：

| 变更类型 | 特征 | 执行路径 |
|----------|------|---------|
| **配置修改** | 仅改 CSV/JSON/SQL/配置文件，无 C++ 代码 | 跳过 3.3 测试、3.4 CMake，编译验证可简化 |
| **代码修改** | 修改/新增 C++ 源文件 | 完整执行所有 Phase |
| **混合修改** | 配置 + 代码都有变更 | 完整执行 |

### 1.3 代码探索

基于提取的关键信息，使用 Grep/Glob/Read 搜索代码库：
- 定位相关模块和入口函数
- 理解现有实现模式和接口
- 识别需要修改或扩展的文件
- **配置修改**：理解配置生成流水线（源文件 → 生成脚本 → 目标文件），确定修改源头

---

## Phase 2：技术方案设计

### 2.1 方案规划

使用 EnterPlanMode 进入方案设计阶段：
- 列出需要修改/新增的文件及其变更要点
- 确定接口设计（新增函数签名、参数、返回值）
- 评估影响范围（哪些现有功能可能受影响）
- 识别风险点
- **配置修改**：明确哪些是源文件（需提交），哪些是生成文件（.gitignore 中，构建时自动生成）

### 2.2 方案确认

将方案摘要展示给用户确认（通过 plan mode 的审批流程）。

> 如果用户传了 `--skip-plan`，跳过此阶段直接进入 Phase 3。

---

## Phase 3：代码实现

### 3.1 分支策略

**默认**：基于 main 创建新分支：
```bash
git checkout main && git checkout -b feat/<简短描述>
```

**`--branch` 模式**：在指定分支上直接工作。实现完成后，如需独立 MR，用 cherry-pick 迁移到基于 main 的新分支：
```bash
# 在当前分支完成实现和提交后
git checkout -b feat/<描述> main
git cherry-pick <commit-hash>...
```

**工作区有未提交修改时**：先临时提交或 stash，切分支后再恢复。

### 3.2 实现原则

- **遵循现有代码风格**：注释语言、命名规范、缩进风格与周围代码一致
- **增量实现**：按方案逐步实现，每步可验证
- **接口优先**：先定义接口/头文件，再填充实现
- **SOLID 原则**：单一职责、开放封闭、依赖倒置

### 3.3 编写测试（代码修改时）

> 配置修改可跳过此步。

测试文件与被测文件同目录，命名 `<模块>_test.cpp`。

**必须覆盖**：
1. **功能测试**：核心功能路径是否正确
2. **边界值**：参数边界、空输入、极端情况
3. **集成点**：与现有模块的交互是否正常

### 3.4 注册到构建系统（新增文件时）

> 仅修改已有文件或配置文件时跳过。

新增文件需要在 CMakeLists.txt 中注册：
- 源文件加入对应 library 的 `SRC_FILES`
- 测试文件用 `ctest_add_test()` 注册

---

## Phase 4：编译与测试验证

**代码修改**：在 Docker 编译容器（`sros-dev`）内完成编译和测试：
```bash
docker exec sros-dev bash -c "cd /workspace/sros-core && ninja -C build-amd64 -j\$(scripts/build_jobs.sh)"
```

**配置修改**：在容器内执行生成脚本验证：
```bash
# 示例：fault_code 配置
docker exec sros-dev bash -c "cd /workspace/sros-core/cfg/sros-resources/bin && python3 generate_fault_code.py"
```

> **注意**：`cfg/sros-resources/target/` 目录属 root 且在 `.gitignore` 中，生成脚本必须在 Docker 容器内执行。本地 host 上无写入权限。

参考 CLAUDE.md 中的编译流程。

---

## Phase 5：提交与 MR

### 提交信息格式

```
#<ONES_ID> feat(<scope>): <一句话描述>
```

### MR 创建

MR 描述应让 reviewer 快速理解**为什么这样改**和**改了什么**：

```bash
glab mr create \
  --title "#<ONES_ID> feat(<scope>): <一句话描述>" \
  --description "$(cat <<'EOF'
## Summary

- ONES: https://ones.standard-robots.com:10120/project/#/team/UNrQ5Ny5/task/<task_uuid>
- <功能概述>

## Background（为什么改）

<问题现象和根因，1-3 句话>

## Design（技术方案与决策）

<方案描述：选择了什么方案、为什么选这个方案>
<如有备选方案被否决，简述原因>

## Changes（改了什么）

**`<文件路径>`**：
- <变更说明>

## Test plan

- [ ] <测试项>
EOF
)" \
  --source-branch <分支名> \
  --target-branch main
```

---

## Phase 6：更新 ONES 状态

除非传了 `--no-upload`，实现完成后执行以下操作：

### 6.1 添加完成评论

评论仅包含结果摘要和 MR 链接，**不包含技术方案或代码细节**（这些在 MR 描述中）：

```python
text = f"""[AI Agent] 处理完成

处理类型：需求实现
完成时间：{now}
处理结果：<一句话描述>
MR：<MR链接>"""
```

使用 python3 urllib 发送评论（模板同 `common.md`）。

### 6.2 状态流转到「产品确认」

通过 transit API 将状态从「开发中」流转到「产品确认」（`3GxUDUn8`）：
```
GET  /team/UNrQ5Ny5/task/<task_uuid>/transitions  → 找 end_status_uuid == 3GxUDUn8 的 transition
POST /team/UNrQ5Ny5/task/<task_uuid>/transit       → {"transition_uuid": "<matched>"}
```

> 如果无法直接流转到「产品确认」（可能需中间状态），按可用 transitions 逐步流转。

### 6.3 设置测试工程师、版本，并回写解决人

transit 完成后，通过 `tasks/update` 设置字段。**必须同时把 `assign` 回写为 ONES token 用户**，避免工作流副作用导致解决人变成他人、统计丢失：

```python
payload = json.dumps({
    "tasks": [{
        "uuid": "<task_uuid>",
        "assign": "<token_user_uuid>",                                    # 回写为 ONES token 用户
        "field_values": [
            {"field_uuid": "Q69TCt3s", "type": 8, "value": "R2bkoeif"},   # 测试工程师 = 魏惠丰
            {"field_uuid": "PL9oJAdQ", "type": 2, "value": "待确认"}       # 软件解决版本号 = 待确认
        ]
    }]
}, ensure_ascii=False).encode("utf-8")
# POST /team/UNrQ5Ny5/tasks/update
```

### ONES 常量速查

| 字段 | UUID / 值 |
|------|-----------|
| 状态「需求导入」 | `5WJAdd7o` |
| 状态「开发中」 | `K7mLx1ej` |
| 状态「产品确认」 | `3GxUDUn8` |
| 测试工程师字段 | `Q69TCt3s`（type=8，用户） |
| 魏惠丰 UUID | `R2bkoeif` |
| 软件解决版本号字段 | `PL9oJAdQ`（type=2，文本） |

---

## 流程检查清单

- [ ] 前置：状态为「需求导入」→ 流转到「开发中」
- [ ] 需求：理解清晰，验收标准明确
- [ ] 方案：用户已确认（或 --skip-plan）
- [ ] 实现：遵循 SOLID，与现有代码风格一致
- [ ] 测试：代码修改时必须有测试；配置修改可跳过
- [ ] 编译/生成：容器内通过
- [ ] 提交：基于 main 的独立分支，commit message 含 ONES ID
- [ ] MR：描述含 Background（根因）+ Design（方案决策）+ Changes + Test plan
- [ ] ONES 评论：结果摘要 + MR 链接（不含技术细节）
- [ ] ONES 状态：流转到「产品确认」，测试工程师=魏惠丰，软件解决版本号=待确认
