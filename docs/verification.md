# 第一阶段验证记录

日期：2026-10-21。验证使用临时仓库、临时 home 和无业务副作用的样例；没有安装当前用户的真实 SM 服务，没有修改现有 Agent 资源，也没有向真实 GitLab 创建测试 MR。

## 自动检查

运行命令：

```sh
.venv/bin/python -m pytest -q
.venv/bin/python -m compileall -q src
node --check src/sm/static/app.js
git diff --check
```

当前 100 项自动检查通过。覆盖模型校验、五类原生投放、受管内容合并、冲突保留、移动清理、MR 隔离、采集去重、候选边界、部分验证、初始化与 Web 边界。`systemd-analyze verify` 对临时生成的 service／timer 通过；systemctl 使用替身，没有真实注册服务。

| 验收 | 证据及边界 |
| --- | --- |
| AC-01 | CLI init／sync、重复初始化、首次 Web 同步、重入锁、生成单元语法检查；未在真实账户启动服务。 |
| AC-02 | 临时 Git 远端 + 真实 Git 分支／合并；待审时同步保持旧内容，合并后的下一次同步才更新。GitLab HTTP 为替身。 |
| AC-03、11 | 两个临时用户目录，同一项目约定，恢复相同内容；项目 Skill 不进入用户目录。不是两台物理机器。 |
| AC-04 | 已批准的重命名、项目→用户移动、移除；旧受管文件删除，无关文件保留；目标冲突时保留旧位置。 |
| AC-05 | 临时上游 Git 提交后直接同步，分析器不被调用，不生成 MR。 |
| AC-06 | 记录入口只依赖标准库，日常不调用模型；完整输出保存为哈希，覆盖缺口明确标注。未对全部原生事件逐一实测。 |
| AC-07、08 | 确定性模型替身贯通“漏触发线索／一次使用两个冗余步骤→分析→候选→前后文本检查→Draft MR”，包含必要脱敏证据。不能据此证明真实模型的诊断质量或业务效率收益。 |
| AC-09 | 无记录不调用模型、无有效变更不提交、同项提案复用、失败保留；不自动合并。 |
| AC-10 | HTTP 和真实 Chrome 浏览器检查五类资源、筛选、阅读、归属、观测统计、证据、MR 入口和运行状态。 |
| AC-12 | 越界路径、HTML 注入、跨来源请求、凭据进入变更／MR 的拒绝检查；不把文件投放成功报告为客户端已加载。 |

## 真实 CLI

Codex 和 Kimi 的分析器分别完成过无业务工具的结构化空候选请求，证明本机认证、调用和回复解析通路可用。这不是资源行为重放。

另用随机名称的被动 Skill 检查共享 `.agents/skills/` 发现能力：Codex 在无工具模式下返回了准确随机名称。Kimi 2.2.0 在临时 Git 项目、仅允许 Read／Skill 只读能力的角色下也返回了准确随机名称；完全无工具的自定义角色未提供技能列表，不能用该结果否定原生发现。两项都是被动元信息检查，未执行 Skill 工作流。Kimi 的[官方 Skill 文档](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/skills.html)列出用户和项目的共享目录支持。

Codex Hook 可能需要原生信任，配置修改也可能要新会话才能加载；SM 展示“文件已应用，Agent 加载状态尚未核验”。Kimi 项目级 Hook 按原生能力限制报告不支持，未提升到用户作用域。

## 真实浏览器

使用 browser-harness 操作本机 Chrome，服务绑定临时 loopback 端口，数据是明确标记的演示资源。MR 提交使用本地替身，链接指向“演示 MR · 未提交远端”。

已检查：八条演示资源覆盖五种类型；项目／Skill 筛选；Skill 内容阅读；创建通用 Skill、修改项目归属、打开 MR；使用证据关联；运行记录；390px 窄屏无整页横向溢出。创建过程中实际发现并修复了旧草稿路径残留，随后重跑创建成功，确认生效目录未变化。

三个前端回归还可用 `SM_BROWSER_URL=<临时服务地址> browser-harness < tests/browser_editor.py` 复现；需要含自建 Skill 的隔离测试目录，所有提交被浏览器拦截。重复名称、草稿改名保留、已有资源重命名均先观察失败，再修复通过。另验证了浏览器实际提交重命名到提案后端、原生效位置保持不变。

截图：[资源目录](screenshots/resources.png)、[使用与改进](screenshots/activity.png)、[窄屏](screenshots/mobile.png)。截图不代表真实用户资源。

## 尚未验证

真实账户 systemd 注册／次日定时运行、真实 GitLab MR 创建和审核合并，以及两台物理机器的恢复尚未执行。真实业务任务、生产 MCP 和机器人部署没有运行。

当前验证为语法与已批准的文本约束，不能替代真实行为对照；优化候选保持部分验证和人工审核。记录只包含原生 Hook 可见的信息，不保证捕获所有 Skill 触发或遗漏；观测事件数量不等于调用次数。原始本地日志暂未设置自动清理。


## 独立审查与修复

整体、Python 和 JavaScript 审查均只读。发现的 10 项问题已完成一次修复，新增回归均观察过失败再通过；未安排重复审查。无暂缓的 Minor 项。

| 发现 | 验证 |
| --- | --- |
| 共享项目目录修改影响未选择的资源 | `test_shared_project_mapping_requires_all_affected_resources_selected` |
| 主分支推进后，更新待审分支发生非快进拒绝 | `test_existing_proposal_updates_after_main_advances`；使用合并保留已发布历史 |
| 复用 MR 的证据和 Draft 状态未刷新 | `test_existing_mr_refreshes_evidence_and_draft_state` |
| 命令／提示词内的凭据未脱敏 | `test_string_credentials_are_redacted_before_recording_and_model_input`；提案入口复用同一识别规则拒绝敏感文本 |
| 格式错误的本地配置中断全部应用 | `test_wrong_config_container_preserves_file_and_other_resources` |
| 错误形状的完整 JSON 事件反复阻塞消化 | `test_wrong_shaped_events_do_not_poison_later_records` |
| 文件执行位更新未应用 | `test_approved_executable_changes_are_applied_with_or_without_content_change` |
| 创建重名资源、改名丢草稿、缺少重命名入口 | `tests/browser_editor.py` 三项断言；二进制附件另由 `test_rename_preserves_binary_assets_and_only_changes_proposal` 覆盖 |

实现裁定和审查边界：

1. 候选支持 edit／retire／reclassify，以覆盖已约定的内容优化、淘汰和组织调整；若接口选择不合适，代价是调整候选 JSON 字段，不会自动改动生效文件。
2. 按用户明确澄清，Skill 采用单一 payload 和共享目录，客户端只负责适配；若将来出现实际兼容限制，代价是调整适配器，无需分叉 Skill 内容。
3. 真实账户服务、原生 Hook 全事件、GitLab 和物理换机未执行；保留为部署验收边界，代价是仍需一次真实环境验收。原生共享 Skill 发现已另行实测。
4. 模型诊断和业务效率保持部分验证、Draft MR 和人工审核；代价是不能据静态检查承诺优化收益。
5. Python／前端专项审查合并进整体结果，使用语法检查、回归和实际浏览器验证；无现成格式／类型检查配置，未作完整辅助技术审计，代价是这些覆盖仍有限。
6. 候选使用修改前的已批准评估约束，没有同次自放宽绕过；后续评估标准的修改仍交 MR 人工审核，代价是审核者需要检查标准本身的变更。

## 交付自评

| 维度 | 分数 | 依据与限制 |
| --- | --- | --- |
| 准确性 | 4/5 | 100 项回归、原生被动发现、浏览器链路有证据；原生 Hook 全事件未实测。 |
| 完整性 | 4/5 | AC-01—12 有对应自动化／边界说明；真实部署和业务效果尚待验证。 |
| 清晰度 | 4/5 | 使用说明区分 MR、投放、加载和行为结果；原生配置仍有工具知识门槛。 |
| 可操作性 | 4/5 | 初始化命令、诊断和完整本地分支已提供；发布与真实账户启用仍需用户决定。 |
| 精简度 | 4/5 | 单 Python 程序、服务端 HTML，无额外前端框架；五类原生格式仍需要适配代码。 |

平均 4.0/5。后续优先真实服务／MR 验收、真实任务前后对照，再补充原生事件覆盖。该评分以用户能否使用及已验证证据为依据，不把暂未执行的部署算作成功。
