# 当前实现与清理验证

核心、CLI、记录、定时任务和 HTTP 服务使用 Rust；浏览器逻辑和回归驱动使用严格 TypeScript。模板、静态文件、分析提示和服务模板嵌入原生二进制。

2026-10-22 按用户要求删除旧实现兼容与确认的冗余代码，只维护当前格式。原迁移设计与计划是历史记录，不再构成数据兼容或回退承诺。

## 已移除

- 旧所有权中间类型、Base64 编解码、binary／selectors 字段，以及重复的写入模式枚举；现在直接持久化类型化 OwnedWrite／WriteBody。
- 模仿 Python 空白分隔符与 Unicode 转义的 JSON 编码器；提案标识和输出摘要使用标准 JSON 序列化。
- 旧状态、事件和提案的迁移夹具与专用兼容测试；替换为当前格式的持久化、去重、Unicode 与损坏数据保护验证。
- 与 Catalog.resources 重复保存的 resources 状态；CLI、Web 和记录处理统一读取 Catalog。
- 没有调用者的访问方法、不会产生的应用状态、内部持久化字段的默认回填，以及 Base64 依赖、Chrono 的未用 serde 功能和 Python 构建忽略项。

不增加旧格式探测、转换器或双读分支。无法识别的所有权或损坏状态报错并保留数据；不会自动清空本机记录或接管无主文件。旧安装不能直接复用，也不能承诺跨实现回退，详见[本地状态说明](usage.md#本地状态)。

## 验证

75 项 Rust 常规回归通过，覆盖四种所有权内容的重启读取、二进制与执行位、私有数据库权限、损坏状态不被清空、原生配置保留、共享目标失败保护、审核后生效、项目作用域、上游不分析、提案去重和七个 CLI 子命令。

新所有权持久化断言先在旧转换层下失败，删除转换层后通过；记录测试先仅提供 Catalog，旧双缓存读取下失败，统一读取后通过。输出摘要只保留哈希，覆盖 Unicode、不同键顺序和内容变化。提案标识覆盖资源顺序无关及原因变化。

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked --bin sm
npm run check
npm run build
git diff --exit-code -- assets/static/app.js
node_modules/.bin/tsc -p tsconfig.browser.json --noEmit
git diff --check
```

TypeScript 额外开启未使用变量／参数检查；产物保持一致。源代码、测试和直接依赖中已无旧所有权格式、迁移编码器或迁移夹具的引用。客户端原生协议适配、可选用户输入、异常处理及安全校验属于现有功能，保留。

整体与 Rust 两项独立审查均无阻断问题；审查指出的 CycleDetails 默认回填已删除，Option 字段多余的 default 注解也已清理，仍保留产品允许省略的输入默认值。

## GitLab CI

`.gitlab-ci.yml` 在 MR 和主线提交时运行两个并行任务。Rust 1.88 任务执行格式、全目标 Clippy、常规测试与 release 构建，并保留可执行文件七天；Node 24 任务通过锁文件安装依赖，执行 TypeScript 严格／未使用项检查、编译、生成产物一致性和浏览器驱动编译。测试容器安装 `systemd-analyze` 所需工具，只验证单元语法，不启动真实用户服务。

构建镜像与 npm 包通过公司现有的 `nexus3.sr` 代理拉取，避免 Runner 依赖直连 Docker Hub 或公共 npm 源。

CI 不提供模型凭据或桌面浏览器；真实模型测试继续默认忽略，桌面 browser-harness 的交互验收单独进行。所有常规任务均须成功才能合并，保留项目的 CI 门禁。

## 验收边界

所有回归使用临时 Git 远端、临时 home 与外部接口替身。没有初始化真实用户服务、删除真实安装数据、执行生产 MCP／机器人任务，也没有验证物理第二台机器。

本次没有重复真实模型请求和浏览器交互；它们在 Rust 迁移时验证过空候选传输与四个前端视角，不代表本次的部署验收。HTTP 流程由九项 Rust Web 回归检查，前端源代码及生成产物未改动。GitLab 的业务 MR 行为仍使用替身，本次代码交付 MR 单独核验。

子进程继承管道的统一超时与共享配置失败时保留旧资产的回归继续保留。优化验证仍限于静态与已批准文本约束，候选保持部分验证与人工审核；原生 Hook 全事件送达、Kimi 任意用户 Hook 隔离和真实业务优化收益仍待实测。
