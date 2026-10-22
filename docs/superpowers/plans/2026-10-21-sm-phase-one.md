# SM Phase One Implementation Plan

> 历史初版计划。当前语言与运行入口以 [Rust 迁移计划](2026-10-21-sm-rust.md) 为准。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 在 Linux 上提供可初始化、可视化的 Codex／Kimi 资源管理工具，完成自建资源经 MR 审核后应用、上游直接同步，以及本地记录和夜间自建资源改进。

**Architecture:** 一个 Python 程序提供 CLI、本地 Web 页面和周期任务；Git 主分支保存资源定义与自建内容，本机保存采集记录和应用状态。资源目录、待审提案和实际生效位置分开，应用逻辑只读取远端主分支的已合并资源。不同 Agent 按实际支持能力生成原生配置。

**Tech Stack:** Python >= 3.10；FastAPI、Uvicorn、Jinja2；tomlkit 保留现有 TOML 配置；markdown-it-py 渲染说明；标准库 SQLite、JSONL、fcntl；Git、glab；systemd 用户服务与 timer；pytest、httpx2。前端使用服务端 HTML 和少量 JavaScript。

**Spec:** [SM 第一阶段需求与行为约定](../specs/2026-10-21-sm-design.md)。以上技术选择是实现方案，不是新增产品要求。

## Global Constraints

- 第一阶段面向 Linux，接入 Codex 和 Kimi。
- 主要服务同一个用户的多台机器；各台机器采用相同的工作目录名、项目命名和组织结构。
- 其他人可以使用同一流程，但必须遵循相同的目录约定。
- 管理 Skill、Hook、规则文件、Agent 定义、MCP 配置。
- 创建方式／来源、作用域、资源类型是独立维度；Skill 不按客户端分类，其他类型单独记录原生适配目标；作用域只考虑用户级和项目级。
- 自建资源统一保存在 SM 仓库，修改 → MR → 人工审核合并 → 下一次本地拉取并应用 → 生效。
- 上游资源本地定时直接跟随上游，不进入自建资源的分析优化队列。
- 第一阶段维护一份当前内容，以主分支为唯一维护来源；不增加多版本选择、并存、发布通道或产品层面的分支管理。
- 日常本地轻量记录，模型分析在周期任务中执行；保留事实、推断和采集缺口的区别。
- 初始化可重复执行，前端默认本机访问；凭据不进入 Git，未核验的接入能力不能报告为成功。

## Review Focus

1. 本地仓库存在未审改动、提案或脏文件时，实际生效内容仍只能来自远端主分支；Task 2、3 验证。
2. 项目名、内容路径或 Agent 能力不匹配时，不扩大作用域、不写出资源边界、不把文件写入报告成 Agent 已接受；Task 1、2、7 验证。
3. 已有规则、Hook、MCP 配置和凭据不能被整文件覆盖；受管内容被本地另改后不能静默覆盖或删除；Task 2 验证。
4. 定时任务重入、采集重复和分析器自身产生的新事件，不能造成重复执行、计数膨胀或重复 MR；Task 3、4、6 验证。
5. 模型提出无证据修改、修改上游资源或要求真实业务操作时，不能自动执行、应用或宣称验证成功；Task 5、8 验证。

---

## 接入事实与实现边界

规划时以 Codex CLI 0.163.0、Kimi Code CLI 2.2.0、Python 3.10.12 核实原生入口；实施后的模型通路、共享 Skill 发现和其他验证结果另见验证记录。

| 资源 | Codex 的原生位置 | Kimi Code 的原生位置 |
| --- | --- | --- |
| Skill | 用户 `~/.agents/skills/`；项目 `.agents/skills/` | 同一份共享 `.agents/skills/` 内容 |
| 规则 | 用户 `~/.codex/AGENTS.md`；项目 `AGENTS.md` | 用户 `~/.kimi-code/AGENTS.md`；项目按其 AGENTS.md 发现规则 |
| Agent 定义 | 用户 `~/.codex/agents/*.toml`；项目 `.codex/agents/*.toml` | 用户 `~/.kimi-code/agents/*.md`；项目 `.kimi-code/agents/*.md` |
| MCP | 用户／项目 `.codex/config.toml` 的 `mcp_servers` | 用户 `~/.kimi-code/mcp.json`；项目 `.kimi-code/mcp.json` |
| Hook | 用户／项目 `.codex/hooks.json` 或对应 TOML 配置 | 文档明确提供用户 `~/.kimi-code/config.toml` 的 `[[hooks]]` |

路径依据：[Codex Skills](https://learn.chatgpt.com/docs/build-skills)、[Codex AGENTS.md](https://learn.chatgpt.com/docs/agent-configuration/agents-md)、[Codex Agents](https://learn.chatgpt.com/docs/agent-configuration/subagents)、[Codex MCP](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)、[Kimi Skills](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/skills.html)、[Kimi Data Locations](https://www.kimi.com/code/docs/en/kimi-code-cli/configuration/data-locations.html)、[Kimi Agents](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/agents.html)、[Kimi MCP](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/mcp.html)。实现时用无副作用样例核对实际加载行为。

- [Codex Hook](https://learn.chatgpt.com/docs/hooks) 有原生信任检查。新建或修改后可能需要用户在 Agent 内信任；SM 不添加绕过信任的启动参数。
- [Kimi Hook](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/hooks.html) 当前只明确了用户配置入口，[Kimi 插件](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/plugins.html)也明确暂不支持项目安装作用域。因此首期把 Kimi 项目 Hook 标记为 `unsupported`，保留管理入口，不擅自移到用户级；若无副作用验证证明当前工具已有原生支持，再启用对应能力。
- Skill 使用一个 `payload`，不含 `agents`／`payloads`，统一投放共享 `.agents/skills/`。其他资源的 `agents` 表达原生适配目标。共享目标内容一致时只写一次，冲突时报告冲突。
- Hook 和会话记录的覆盖不是所有模型行为的完整证明。使用次数基于可见的调用／读取证据；仅出现在可用列表中不算使用。

## 文件与数据约定

| 文件／目录 | 职责 |
| --- | --- |
| `pyproject.toml`、`sm` | 安装描述、`./sm init` 引导入口及 CLI |
| `catalog.json`、`resources/<id>/` | 项目目录约定、资源元数据和自建内容；上游条目只保存获取定义 |
| `src/sm/catalog.py`、`paths.py` | 校验资源与项目归属，解析当前用户下的约定目录 |
| `src/sm/providers/codex.py`、`kimi.py` | 原生配置与能力判断，无通用插件框架 |
| `src/sm/repository.py`、`apply.py`、`state.py` | 获取已合并内容、生成／执行受管修改、本机 SQLite 状态 |
| `src/sm/proposals.py`、`gitlab.py` | 待审内容、普通 Git 提案分支、规范 MR 与重复提案识别 |
| `src/sm/observe.py`、`analysis.py`、`evaluation.py` | 轻量采集、自建资源分析和受控对比验证 |
| `src/sm/cli.py`、`service.py` | CLI、任务锁、初始化与 systemd 配置 |
| `src/sm/web.py`、`templates/`、`static/` | 本地前端及交互 |
| `tests/fixtures/`、`tests/test_*.py` | 原生格式、临时 Git 远端、事件、提案与验收样例 |

`catalog.json` 顶层为 `projects` 和 `resources`。`projects` 保存约定项目名到目录的对应关系，不增加个人目录映射。共同资源字段为 `id`、`kind`、`origin`、`scope`、`project` 和可选 `upstream`。Skill 使用单一 `payload`；其他类型使用 `agents` 和 `payloads` 表达客户端原生适配。

`upstream` 使用 Git URL／默认分支获取，或调用仓库内明确登记的生成脚本；生成脚本接收暂存输出目录，内容校验后才交给应用逻辑。程序不提供任意远程脚本发现或插件市场。内容路径必须留在对应来源内部。

本机数据存于 XDG state/data/config 目录下的 `sm/`：JSONL 记录事件，SQLite 保存受管目标、内容摘要、处理进度、提案关联及执行结果。摘要用于识别实际内容和本地冲突，不提供资源版本选择。测试传入临时路径，不改写真实 HOME 或 Agent 配置。

所有任务按“编写行为测试 → 确认失败 → 最小实现 → 确认通过 → 提交”执行。测试使用临时目录、临时 Git 远端和 CLI 替身；真实服务、模型、外部 MR 与 Agent 加载验证另列结果。

### Task 1: 资源目录、作用域和原生配置预览

**Files:** Create `pyproject.toml`, `src/sm/__init__.py`, `src/sm/catalog.py`, `src/sm/paths.py`, `src/sm/providers/__init__.py`, `src/sm/providers/codex.py`, `src/sm/providers/kimi.py`, `catalog.json`, `tests/test_catalog.py`, `tests/fixtures/native/`。

**Interfaces:** 公共类型在 `catalog.py` 定义：`Resource` 表达上述固定字段；`Catalog.projects: dict[str, str]`, `Catalog.resources: list[Resource]`。`load_catalog(root: Path) -> Catalog`；`resolve_scope(resource: Resource, catalog: Catalog, home: Path) -> Path`；两个 provider 各提供 `render(resource: Resource, payload: Path, scope_root: Path) -> list[OwnedWrite]`。`OwnedWrite` 携带 `resource_id: str`, `path: Path`, `mode`（file／markdown_block／toml_entries／json_entries）, `content: bytes | dict[str, object]`, `selectors: list[str]`；目录内容展开为逐文件写入。不兼容的组合抛出同文件定义的 `UnsupportedResource`。

- [x] 建立本任务所需的虚拟环境、最小包配置和测试依赖；后续 `python` 命令均在该虚拟环境执行，不提前搭建其余功能。
- [x] 编写测试：五类资源可加载；项目级无项目归属失败；未知 Agent 失败；`../`、绝对内容路径和越界符号链接失败；同一生效目标内容冲突失败；Kimi 项目 Hook 返回明确不支持。

```python
def test_project_scope_requires_project(tmp_path):
    resource = dict(id="review", kind="skill", origin="self", scope="project",
                    project=None, payload="resources/review")
    (tmp_path / "catalog.json").write_text(json.dumps({"projects": {}, "resources": [resource]}))
    with pytest.raises(ValueError, match="project"):
        load_catalog(tmp_path)
```

- [x] Run `python -m pytest tests/test_catalog.py -q`，确认行为测试在实现前失败。
- [x] 实现上述接口和原生格式样例。配置内容采用对应 Agent 的原生格式，保留自建和上游来源；共享 Skill 目标去重，依赖和安装入口与本任务一起提交。
- [x] 重跑上述命令，要求全部通过；核对项目级目标没有用户级路径。
- [x] Commit `feat: define resource catalog and native targets`。

### Task 2: 只应用已合并内容，保护现有本地配置

**Files:** Create `src/sm/repository.py`, `src/sm/apply.py`, `src/sm/state.py`, `tests/test_sync.py`, `tests/test_apply.py`；使用 Task 1 的接口。

**Interfaces:** `fetch_approved(repo: Path, output: Path) -> Path` 从 `origin/main` 导出目录定义和资源供只读处理；`fetch_upstream(resource: Resource, output: Path) -> Path` 返回暂存内容；`plan_apply(catalog: Catalog, sources: dict[str, Path], home: Path, state: State) -> list[OwnedWrite]`；`apply_writes(writes: list[OwnedWrite], state: State) -> ApplyReport`。`state.py` 定义 `State(path: Path)` 保存归属与运行结果，以及 `ApplyReport`：每份资源的结果（applied／needs_agent_trust／next_session／unsupported／conflict／failed）、目标和可读原因。`applied` 只表示受管文件应用完成，原生加载结果单独说明。

- [x] 编写临时 Git 远端测试：远端 main、未合并提案和本地脏内容各有不同文本，只有远端 main 被应用。上游获取更新不产生 MR，不进入分析输入。
- [x] 编写应用测试：保留已有 AGENTS.md 的非受管段落、现有 Hook、MCP 其他条目及凭据；重命名／移动／删除仅清理受管目标；发现受管目标被另改时报告冲突；写入中断不留下截断配置。

```python
def test_rule_application_preserves_existing_text(tmp_path):
    target = tmp_path / "AGENTS.md"
    target.write_text("已有约定\n")
    write = OwnedWrite(resource_id="review", path=target, mode="markdown_block",
                       content="新增约定\n".encode(), selectors=["review"])
    apply_writes([write], State(tmp_path / "state.sqlite3"))
    assert target.read_text().startswith("已有约定\n")
    assert "新增约定" in target.read_text()
```

- [x] Run `python -m pytest tests/test_sync.py tests/test_apply.py -q`，确认失败。
- [x] 实现 Git 获取和暂存校验，再原子替换完整文件或合并受管条目。Markdown 使用带资源 ID 的受管段落；TOML／JSON 只更新登记归属的选择器。拒绝覆盖同名非受管内容；缺少目标项目时报告未应用，不自动创建假项目。应用账本仅在对应写入完成后更新。
- [x] 重跑测试要求全部通过；确认无需切换或清理用户的工作分支。
- [x] Commit `feat: sync approved resources without clobbering local config`。

### Task 3: 日常编辑、组织调整与规范 MR

**Files:** Create `src/sm/proposals.py`, `src/sm/gitlab.py`, `tests/test_proposals.py`；更新 `state.py`。

**Interfaces:** `proposals.py` 定义 `ChangeSet`，字段为 `resource_ids: list[str]`, `reason: str`, `evidence: list[str]`, `writes: dict[str, str]`（仓库相对路径到新文本）, `deletions: list[str]`, `validation: dict[str, object]`；`create_proposal(repo: Path, change: ChangeSet, state: State) -> Proposal`；`gitlab.py` 提供 `submit_mr(proposal: Proposal) -> str`。`Proposal` 携带本地待审目录、提案标识、关联 MR 和处理状态，不包含本地应用动作。

- [x] 编写测试：新建、编辑、改名、改归属和淘汰只改待审内容；运行位置保持不变；伪造越界文件或修改上游内容失败；相同问题复用已存在待审提案；提交失败不标记已提交。

```python
def test_changeset_rejects_parent_path():
    with pytest.raises(ValueError, match="path"):
        ChangeSet(resource_ids=["review"], reason="精简步骤", evidence=["event-1"],
                  writes={"../AGENTS.md": "越界内容"}, deletions=[], validation={})
```

- [x] Run `python -m pytest tests/test_proposals.py -q`，确认失败。
- [x] 在隔离的普通 Git 提案目录准备修改，沿用正常 MR 分支流程；通过 argv 调用 Git／glab。MR 请求包含 source_branch、target_branch、title、description，完整 JSON 写文件，用 `glab api --method POST --input <file> --header 'Content-Type: application/json'` 提交到项目 merge_requests 端点。创建后读取并核对结果；重试前检查既有来源分支对应的 MR。没有安全行为验证时使用草稿 MR 并列明缺口，不伪造通过结果。
- [x] 重跑测试全部通过；检查测试替身收到的正文保留换行，MR 创建不会调用合并或本地应用。
- [x] Commit `feat: propose resource changes through GitLab review`。

### Task 4: 本地轻量记录与采集覆盖展示

**Files:** Create `src/sm/observe.py`, `tests/test_observe.py`, `tests/fixtures/events/`；更新两个 provider 的 SM 采集配置。

**Interfaces:** `record_event(agent: str, payload: dict, state_dir: Path) -> None` 只追加本地 JSONL；`ingest_events(state_dir: Path, state: State) -> list[Observation]`。`observe.py` 定义 `Observation`：`agent`, `session_id`, `cwd`, `event`, `timestamp`, `resource_id: str | None`, `resource_content_hash: str | None`, `evidence_kind`, `facts: dict[str, object]`, `coverage`。使用证据区分明确调用、可见内容读取、执行证据与推断，摘要仅在有对应内容证据时填写。

- [x] 编写测试：Codex／Kimi 样例事件可记录；上游不进入自建分析；仅可用列表不增加使用次数；记录不全显示 unknown；重放相同事件不重复计数；`SM_INTERNAL_RUN=1` 的分析器事件不再次进入分析。

```python
def test_internal_analysis_does_not_record_itself(tmp_path, monkeypatch):
    monkeypatch.setenv("SM_INTERNAL_RUN", "1")
    record_event("codex", {"session_id": "internal", "hook_event_name": "Stop"}, tmp_path)
    assert list(tmp_path.rglob("*.jsonl")) == []
```

- [x] Run `python -m pytest tests/test_observe.py -q`，确认失败。
- [x] 实现有长度边界的追加记录，记录入口只加载标准库，不导入 Web／分析模块，不在 Hook 中调用模型或上传记录。配置 SM 自身的用户级采集 Hook，保留原有 Hook；stdout 保持空，采集失败记本机诊断而不改变业务 Hook 的返回行为。需要原生信任时明确记录采集未启用。
- [x] 重跑测试并测量记录器调用开销；报告测量条件与结果，不编造毫秒承诺。证明进程调用中没有分析器命令。
- [x] Commit `feat: collect local resource usage with explicit coverage`。

### Task 5: 自建资源分析、对比验证和优化提案

**Files:** Create `src/sm/analysis.py`, `src/sm/evaluation.py`, `src/sm/prompts/analyze.md`, `src/sm/analysis.schema.json`, `tests/test_analysis.py`, `tests/fixtures/analysis/`。

**Interfaces:** `analysis.py` 定义 `analyze_observations(catalog: Catalog, observations: list[Observation], runner: AgentRunner) -> list[Candidate]`、`AgentRunner.run(prompt: str) -> dict` 及 `CodexRunner`／`KimiRunner`。同文件的 `Candidate` 包含 `resource_id: str`, `issue: str`, `evidence: list[str]`, `changes: dict[str, str]`, `expected_checks: list[str]`，以及 `operation`（edit／retire／reclassify）、`scope`、`project`。`evaluation.py` 定义 `evaluate_candidate(candidate: Candidate, original: Path, proposed: Path, runner: AgentRunner) -> ValidationReport`；报告包含 passed／failed／partial 状态、检查结果和未验证项。`to_changeset(candidate: Candidate, report: ValidationReport) -> ChangeSet` 在 `analysis.py` 实现。

- [x] 编写测试：一次使用发现两项冗余可生成带证据提案；低频但无适用任务不直接淘汰；缺少记录不捏造漏触发；证据内容与当前内容不符时保留观察但不直接打补丁；拒绝上游资源 ID、无证据修改、格式错误与越界写入；没有候选时返回空列表。
- [x] 编写对比样例：冗余读取删除后仍完成任务，必要复查被误删则验证失败；两者必须有不同结果。涉及真实部署、硬件、远程写入的验证不自动执行，报告未验证原因。

```python
def test_empty_observations_do_not_call_a_model():
    runner = Mock()
    assert analyze_observations(Catalog(projects={}, resources=[]), [], runner) == []
    runner.run.assert_not_called()
```

- [x] Run `python -m pytest tests/test_analysis.py -q`，确认失败。
- [x] 分析器默认复用可用的本机 CLI。Codex 使用非交互、只读 sandbox 和结构化输出；Kimi 使用 `-p`、`stream-json` 及 `tools: []` 的专用分析 Agent。输出先按 schema 校验，再由 SM 生成受限文件改动；不执行模型提供的任意命令，也不添加绕过权限或 Hook 信任的参数。验证先提供静态检查与无副作用案例回放，报告明确区分回放、实际行为复跑与未验证项，不把轨迹推断当成真实复跑结果。
- [x] 使用替身验证完整链路后，在明确的无外部副作用样例上分别做真实 CLI 验证并记录费用／失败情况；没有真实模型证据时结果明确标注替身验证。依据验证结果使用 Task 3 提交提案，人工仍负责审核合并。
- [x] Commit `feat: turn self resource observations into reviewed improvements`。

### Task 6: 一个初始化入口、首次同步和夜间任务

**Files:** Create `sm`, `src/sm/cli.py`, `src/sm/service.py`, `src/sm/systemd/sm-web.service`, `src/sm/systemd/sm-cycle.service`, `src/sm/systemd/sm-cycle.timer`, `tests/test_service.py`；更新 `pyproject.toml` 的 CLI 入口。

**Interfaces:** `service.py` 定义 `initialize(repo: Path, home: Path) -> InitReport`、`synchronize(repo: Path, home: Path, state: State) -> ApplyReport`（组合 Task 1、2、4）、`cycle(repo: Path, home: Path, state: State, runner: AgentRunner) -> CycleReport`（先消化现有记录，再完成同步与自建分析提案）和 `job_lock(state_dir: Path)`（本机非阻塞文件锁）。同文件的 `InitReport`／`CycleReport` 包含每阶段结果和未完成原因。CLI 在入口解析当前用户目录，再传入这些接口；固定入口为 `init`, `serve`, `sync`, `cycle`, `record`, `status`, `propose`。

- [x] 编写测试：第一次 init 创建配置和用户服务；第二次不重复 Hook／任务；缺依赖或 systemd 用户会话不可用时给出明确结果；服务首次运行触发同步；定时重入只执行一份；单个资源失败仍有逐项结果，未完成阶段不会被记为已处理。

```python
def test_cycle_lock_prevents_reentry(tmp_path):
    with job_lock(tmp_path):
        with pytest.raises(RuntimeError, match="already running"):
            with job_lock(tmp_path):
                pass
```

- [x] Run `python -m pytest tests/test_service.py -q`，确认失败。
- [x] `./sm init` 建立本地虚拟环境和 CLI，登记仓库位置、安装用户服务／timer。默认本地时间 02:00 执行周期任务，使用 systemd 的持久 timer；Web 首次启动调度一次同步。账号与外部工具复用现有环境，检查失败给出具体准备项。服务日志和结果写入本机状态，`SM_INTERNAL_RUN` 传给分析子进程。
- [x] 重跑测试；用 `systemd-analyze --user verify` 核对生成单元。仅在用户明确运行真实初始化时注册当前账户服务，开发测试使用临时配置和 systemctl 替身。
- [x] Commit `feat: initialize local services and nightly resource jobs`。

### Task 7: 面向资源、项目、改进和运行状态的前端

**Files:** Create `src/sm/web.py`, `src/sm/templates/base.html`, `resources.html`, `resource.html`, `projects.html`, `activity.html`, `jobs.html`, `src/sm/static/style.css`, `src/sm/static/app.js`, `tests/test_web.py`；更新 `cli.py` 的 `serve`。

**Interfaces:** `create_app(repo: Path, home: Path, state: State) -> FastAPI`；GET `/resources`, `/resources/{id}`, `/projects`, `/activity`, `/jobs`；POST `/proposals` 接收 Task 3 的经过校验的变更，返回提案／MR 状态。浏览器从资源 ID 操作，不接收任意本地文件路径。服务默认绑定 `127.0.0.1:8765`。

- [x] 编写测试：五类资源可读，来源／作用域／Agent 筛选正确；项目页只展示对应归属；内容渲染禁用原始 HTML，脚本字符串被转义；来源不支持、待信任、未采集、应用失败各有明确状态；前端发起修改只产生待审提案。
- [x] 编写边界测试：跨来源或不合格 Host 的修改请求被拒绝，路径不能越界，本机凭据和原始会话不出现在页面；没有采集证据时不展示“零使用＝可淘汰”。

```python
def test_foreign_origin_cannot_create_a_proposal(tmp_path):
    (tmp_path / "catalog.json").write_text('{"projects": {}, "resources": []}')
    app = create_app(tmp_path, tmp_path, State(tmp_path / "state.sqlite3"))
    with TestClient(app, base_url="http://127.0.0.1:8765") as client:
        response = client.post("/proposals", json={}, headers={"Origin": "https://other.invalid"})
    assert response.status_code == 403
```

- [x] Run `python -m pytest tests/test_web.py -q`，确认失败。
- [x] 实现服务端页面、关联导航和必要交互；资源页面提供内容阅读、创建／编辑／归属调整／淘汰的提案入口。MR 评审继续打开 GitLab，资源应用结果和采集状态按实际证据展示。修改请求校验 Origin／Host 和本地 CSRF token。
- [x] 重跑测试；使用 browser-harness 做真实浏览器检查：浏览资源、切换作用域、查看问题证据、发起提案、打开 MR、查看运行结果；保留截图和交互记录，不把 HTTP 测试当浏览器证据。
- [x] Commit `feat: add local visual resource management`。

### Task 8: 换机恢复与第一阶段交付验收

**Files:** Create `tests/test_acceptance.py`, `tests/conftest.py`, `tests/fixtures/acceptance/`, `docs/usage.md`, `docs/verification.md`；更新 `README.md`。

**Interfaces:** 复用上述 CLI 和 Web，不新增产品接口。验收夹具包含临时 main 远端、待审变更、两个同目录约定的临时用户环境、Gitlab／Agent／systemd 替身和无副作用原生格式样例。

`tests/conftest.py` 的 `two_machine_case` 夹具提供 `repo: Path` 和 `homes: tuple[Path, Path]`，并配置外部调用替身。远端 main 含一个仅属于 `app` 项目的 `review` Skill；项目约定为 `~/repo/app`，两个临时 home 中均已有该项目目录。测试不改真实 HOME，CLI 测试通过接口传入临时 home。

- [x] 编写端到端用例，覆盖需求 AC-01—AC-12：单命令初始化、未审内容隔离、跨机作用域一致、组织调整清理、上游更新、自建使用分析、冗余改进 MR，以及凭据保留。

```python
def test_two_machine_restore_keeps_project_scope(two_machine_case):
    for home in two_machine_case.homes:
        initialize(two_machine_case.repo, home)
        synchronize(two_machine_case.repo, home, State(home / ".local/state/sm/state.sqlite3"))
    a, b = two_machine_case.homes
    relative = "repo/app/.agents/skills/review/SKILL.md"
    assert (a / relative).read_bytes() == (b / relative).read_bytes()
    assert not (a / ".agents/skills/review").exists()
    assert not (b / ".agents/skills/review").exists()
```

- [x] Run `python -m pytest tests/test_acceptance.py -q`，确认失败；补齐前述任务之间缺失的连接，不另建演示用旁路。
- [x] 完成全套测试：`python -m pytest -q`；静态检查：`python -m compileall -q src`、`git diff --check`。需要新增回归时再扩大测试，不无目的重复。
- [x] 在隔离环境验证真实 Codex／Kimi 的安全加载样例，记录原生信任与下一会话要求；核对 Kimi 项目 Hook 仍按能力限制显示。不会自动运行现有机器人部署、生产 MCP 操作或其他业务副作用。
- [x] 文档提供 clone 后的唯一初始化命令、前端入口、普通日常编辑流程、夜间行为和诊断入口；验证报告分别列出替身／临时目录检查、真实 CLI、真实浏览器、真实用户服务和外部 MR 结果，缺项明确列出。
- [x] Commit `test: verify SM initialization and reviewed resource lifecycle`。

## 需求覆盖与执行自查

| 需求 | 实现任务 |
| --- | --- |
| 五类资源、多维分类、用户／项目作用域、统一目录 | 1、2、7 |
| 自建内容集中维护、未审内容隔离、组织变更 | 2、3、7 |
| 上游直接跟随且不分析 | 2、4、5 |
| 使用情况、漏触发、一次使用后的反思 | 4、5 |
| 规范 MR、人工合并、无依据不提交 | 3、5 |
| 一个 init、首次执行、夜间同步分析 | 6 |
| 多视角前端、关联状态与证据 | 7 |
| 换机恢复、真实接入验证及所有 AC | 8 |

执行顺序为 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8。接口依赖较多，建议由当前 Agent 连续实现，并在完成分支后做独立审阅；若用户选择逐任务的子 Agent 实施与审阅，再按指定方式执行。

实施结果和真实接入验证边界见 [验证记录](../../verification.md)。原生目录发现、文件应用、行为验证分别报告。
