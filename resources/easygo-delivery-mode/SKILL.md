---
name: easygo-delivery-mode
description: EasyGo 任务最快的正确交付路径。开工前先分类（前端/后端/仿真/硬件），选宿主直做 vs 容器 easygo-dev-main vs 代码+测试+MR 绕过环境墙；按比例验证（单测/静态 vs 仿真）；环境/硬件阻塞明确上报不烧重试；控制成本（避免精确重复探测，一次精准查询）。触发词：EasyGo、easygo、后端构建、仿真、环境阻塞、前端任务、交付、无法定位。
---

# EasyGo 交付模式

来自 90+ 个 EasyGo 真实 run 的观测沉淀。开工**先分类、再选路径**，避免空转与踩坑。

## 1. 任务分类与路径选择

| 任务类型 | 路径 |
|---|---|
| 纯前端（easygo-fe：地图编辑/时间选择器/同步进度/弹窗等） | **宿主直做**：隔离 worktree → 定位组件 → pnpm/vitest 单测或静态证据 → 仓库流程提 MR。不碰 ROS build |
| 后端 easygo-app / ROS 相关 | 容器 **easygo-dev-main**（jazzy）。宿主 /opt/ros 只有 humble，直接 cargo build 必失败 |
| 环境/依赖墙（见 §4） | **代码+测试+MR 交付模式**，或明确上报跳过，不烧重试 |
| 仿真链路（Gazebo/导航/多车） | full-simulation 技能；非必要不起全量仿真 |

## 2. 验证决策（按比例，不全量仿真）

- 纯函数/静态/单测足够 → 跑单测/静态证据，并在交付中说明。
- 跨组件/Gazebo/真实用户链路 → 才起仿真（world=caterpiller_cad，headless 用 use_gz_gui:=false use_rviz:=false）。
- 多车仿真内存要求高（≥24GB），默认单车。
- 交付必须含可核验的运行方式/命令（delivery report），不要只交"改了代码"。
- **delivery report JSON 必须含 `run` + 逐条 `items`**（高频被拒教训，记忆 #17）：run = url 或 {command,port}；items = 每条验收 criterion 配 check（interact/command/evidence）。纯验证/复跑/结论型交付同样要给：run = 验证命令，items = 每条验收的 evidence/command check。提交前自检。

## 3. 后端环境墙的绕过模式

SZ-2 地图同步定位重置等已验证：不依赖缺失的 ROS msg 包、不起全量仿真，在隔离 worktree 改代码 + 补插件级回归测试（等价覆盖数据一致性）+ 开真实 MR（easygo-app 仓），如实说明全量仿真需 MR 合并后重起。

## 4. 已知环境阻塞（确证后上报，勿空转）

- **easygo-follow-msgs 0.3.11 版本墙**：外部 ROS msg 包（SROS/core/follow.git），crates.io/rsproxy 无、容器里是 0.3.5、easygo-app 要 0.3.11。cargo 解析必报版本错。需构建 0.3.11 进容器 /opt/sros 并补 patch 才能解。
- 硬件类（EVT WiFi 模块 SDIO 枚举、电池通讯、驱动器力矩限幅等）→ 定位到硬件/驱动器侧则给分流结论，不烧软件重试。

## 5. 成本纪律（kimi/deepseek-v4-flash 逐调用记账）

- 每次 model 调用都重读整段上下文 → **cache_read 是成本主因**（一个 200-350 步任务可达 5-28M cache-read tokens）。
- 一次精准查询胜过整串重复探测：工具调用 ~50% 是精确重复，直接放大 cache_read。
- 优先小范围精准读（文件行区间/grep 定位），避免整文件反复重读。
- **结构查询先 codegraph**：找定义/调用/影响面用 codegraph MCP（search/trace/explore/callers），一次亚毫秒查询胜过一串 grep；Easygo 主索引已就绪且从 worktree 自动解析。仅字面文本才用 Grep。

## 5.5 动工前去重（收敛/复跑必读）——按此步骤，实测比一句规则省 15-45%

开 MR/实现前**按步骤执行**（记忆 #14 实测：detailed 步骤化措辞比"先查有则复查不重开"省 15-45% 成本与步数）：
1. **搜**：用 gitlab 按本 issue/根因关键词搜 open 与 merged MR（标题/分支含 fix/feat+功能词）。
2. **有等价 MR** → 读其 diff 与测试，独立验证是否完整覆盖本任务根因与验收；交付 = 复查报告（引用 MR id + CI/pipeline 状态 + 明确结论），**不要新实现、不要新开 MR**。
3. **确认无** → 才实现并新开 MR；MR 描述列出同根因已有 MR（例：与 !565 等价可择优）。

多 agent 并行同契约时先读共享"已开 MR 清单"。教训：同一根因曾出现 !577/!579/!565 三份、!452/!470 双份；rebase 时若发现内容已在上游，不 force-push 清空 diff，报告并建议关闭。

## 7. 提 MR 描述规范（开新 MR 必读）

MR 描述必须**结构化四段 + 溯源链接**，评审者据此核对，缺项会被退回（记忆 #17/#24 教训）。

### 固定结构
1. **问题（Problem）**：一句话缺陷现象/需求，含影响面（谁在什么场景遇到）。
2. **根因（Root cause）**：代码级定位——文件:行/函数 + 触发链路；能给出引擎/单测级复现证据的写复现（如 `FAILED([...])`、日志序列）。
3. **修复（Fix）**：改了什么、为什么这样改能消除根因；列出改动文件；若与既有 MR 同根因择优，注明关系。
4. **验证（Verification）**：跑过的测试/命令与结果（单测 N/N、CI pipeline id、静态检查），以及未覆盖项与原因。

### 溯源链接（Description 尾部固定加 "Traceability" 块）
- **Organism 任务**：`Organism task #<task_id>`（本任务 id，必填）+ 交付 run 链接（delivery report 的 run.url/命令）。
- **ONES 来源**：从任务描述提取 ONES 工作项链接（`https://ones.standard-robots.com:10120/.../SC...`）放进 Traceability；非 ONES 来源（git 探子/手工）标注来源即可。

### 反例（被退回的真实 MR）
- 只有一句话"fix xx"无根因/验证 → 评审无法核对。
- 缺 run 方式 / 缺逐条验收 items → 交付被拒（记忆 #17）。
- 不提同根因既有 MR → 造成重复 MR（!577/!579/!565 教训）。

## 6. 已定位/已解决问题索引（查 guide，别重做）

已解决/已定位问题与其 MR 的完整索引见 **guide 资产 `easygo-resolved-issues`**（随本任务注入）。动工/复跑前先查它命中项，命中则去重复查给结论，不重开 MR。
