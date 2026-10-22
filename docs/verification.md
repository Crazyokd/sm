# 当前验证与边界

核心、CLI、记录、定时任务和 HTTP 服务使用 Rust；浏览器逻辑和原生插件桥接使用严格 TypeScript。只维护当前实现和格式；原型与迁移过程以 Git 历史和历史文档为准。

以下为开发侧验证。本机当前覆盖与运行情况以“纳管覆盖”“运行状态”和交付 MR 的流水线为准。

## 自动检查

182 项 Rust 回归通过，0 项失败，1 项需要真实模型凭据的测试默认忽略。覆盖审核指纹接管、路径与符号链接边界、共享目标失败保护、原生配置保留、来源验证、禁用状态、嵌套规则、遗漏与漂移检查、提案审核、本机私有值绑定和记录保留。

启动回归验证：联网刷新前完成本地自维护内容应用和采集索引；完整计划就绪前保留旧资源。MCP 私有值缺失不会阻塞无关规则；插件启停页面只提交 MR，不立即改变安装状态。

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked --bin sm
npm run check
npm run build
git diff --exit-code -- assets/static/app.js resources/source-providers/native-plugins.js
node_modules/.bin/tsc -p tsconfig.browser.json --noEmit
git diff --check -- src frontend assets tests .gitlab-ci.yml
```

导入资源保留原始字节与执行位，包括 Markdown 硬换行所需空格，不通过代码格式化改写其内容。

## 真实来源与目录预检

实际通过原生安装器生成 ECC 2.2.3 资源：1,066 个唯一目标，其中 488 个额外文件的内容哈希与执行位均匹配发布包，没有缺失来源。验证没有执行这些资源描述的业务流程。

在本机目录与 SQLite 状态副本上预检 10 个项目、131 项资源声明和 9 项外部原生安装：共识别 1,670 项组件，未纳管 0、本地漂移 0。三个悬空旧链接仅在路径、链接内容和悬空状态都匹配审核清单时清理。预检不等于本机部署验收。

真实 Kimi 本地接口的只读检查返回 3 个插件均处于正常状态；探针没有调用安装、模型或业务会话。原生更新失败、启停及子进程清理仍主要由隔离接口替身验证，实际更新结果由本机运行日志单独报告。

## CI 与验收边界

MR 和主线执行 Rust 1.88 与 Node 24 检查。Rust 容器安装 Node.js 以运行桥接替身测试；TypeScript 任务检查两个严格配置及生成 JavaScript 一致性。CI 没有模型凭据和桌面浏览器，不启动真实用户服务。

部分 GitHub 来源的原生克隆已出现 300 秒超时。刷新失败单独报告，已审核的本地内容继续受管；这不代表上游已经更新成功。客户端内置资源由平台维护，SM 展示并区分其来源。

第二台物理机器恢复、原生 Hook 全事件送达和真实业务优化收益仍待实测。资源改进保持证据、MR、人工审核合并、下次拉取应用的流程。
