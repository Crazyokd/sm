# easygo 项目 AGENTS 文档

## 1. 文档目标

本文档面向在 `easygo` 仓库协作的开发者与 AI Agent，提供可执行的项目上下文，避免继续沿用 Vue 模板时期的信息。

适用范围：仓库根目录及 `src/` 主应用代码、`__generated__/` 生成代码、`docker/` 发布配置。

---

## 2. 项目现状快照（2026-04-22）

- 项目类型：Vue 3 + TypeScript + Vite 8 的业务前端（AGV 工控场景）
- 路由方式：`vue-router/auto-routes` 文件路由 + `vite-plugin-vue-layouts` 布局注入
- 样式方案：UnoCSS（`virtual:uno.css`）
- 数据层：`@tanstack/vue-query` + `axios`（OpenAPI 生成客户端）
- 代码质量：TypeScript 严格模式 + ESLint（`@antfu/eslint-config`）
- 测试现状：`pnpm test` 仍是占位命令，暂无自动化测试落地

---

## 3. 关键技术栈

| 类别 | 方案 |
| --- | --- |
| 前端框架 | `vue@3.5.x` |
| 构建工具 | `vite@8.0.0-beta.13` |
| 语言 | `typescript@5.9.x` |
| 路由 | `vue-router@5` + `vue-router/vite` 自动路由 |
| 状态请求 | `@tanstack/vue-query` |
| UI 能力 | `@modalor/vue`、`vue-sonner`、`reka-ui`、`@remixicon/vue` |
| 样式 | `unocss`（`presetWind4` + `presetTheme`） |
| API 生成 | `@hey-api/openapi-ts`（输出到 `__generated__/api-clients/EasyGo`） |

---

## 4. 目录与职责

```text
easygo/
├── src/
│   ├── pages/                 # 页面路由（自动路由入口）
│   │   ├── map/               # 地图编辑核心模块（core/data/features/shared/operators）
│   │   ├── station/           # 站点管理
│   │   ├── navigation/        # 导航管理
│   │   ├── scene/             # 场景与巡检流程
│   │   ├── settings/          # 设置页
│   │   └── system/            # 系统页（语言等）
│   ├── layouts/               # default / empty 布局与 RobotHeader
│   ├── components/            # 通用组件、业务卡片、通用弹窗
│   ├── api/                   # API 客户端接入与统一拦截
│   ├── modules/               # 应用级插件模块（当前含 vue-query 安装）
│   ├── composables/           # 组合式逻辑（任务状态上下文）
│   ├── constants/             # 常量定义
│   ├── data/                  # 本地演示/静态数据
│   ├── utils/                 # 通用工具
│   ├── router.ts              # 路由创建 + HMR 更新
│   ├── main.ts                # 应用启动入口
│   └── App.vue                # 根组件（RouterView、Modalor、Toaster）
├── __generated__/api-clients/ # OpenAPI 生成代码（禁止手改）
├── docker/                    # 镜像构建、nginx 模板、compose 模板
├── scripts/minio.js           # dist 打包并上传 MinIO
├── docs/DESIGN_SYSTEM.md      # 设计系统约束（工业平板场景）
└── AGENTS.md                  # 本文档
```

---

## 5. 启动链路与运行机制

1. `src/main.ts`
   - 创建 Vue App
   - 注册 `router`
   - 自动加载 `src/modules/*.ts` 并执行 `install`
2. `src/router.ts`
   - 基于 `routes`（auto-routes）创建路由
   - 通过 `setupLayouts` 注入布局
3. `src/App.vue`
   - 渲染 `RouterView`
   - 挂载全局弹窗容器 `Modalor`
   - 挂载全局通知 `vue-sonner`
4. `src/api/index.ts`
   - 对 EasyGo API 客户端配置响应拦截
   - 统一展示业务错误提示（toast）

---

## 6. 开发命令（以 `pnpm` 为准）

```bash
pnpm dev            # 本地开发（--host）
pnpm build          # vue-tsc -b + vite build
pnpm preview        # 预览构建结果
pnpm lint           # ESLint 检查
pnpm lint:fix       # ESLint 自动修复
pnpm fmt            # oxfmt 格式化
pnpm fmt:check      # oxfmt 检查
pnpm typecheck      # vue-tsc
pnpm openapi-ts     # 根据 openapi-ts.config.ts 生成客户端
pnpm openapi-ts:fetch # 拉取内网 OpenAPI 后再生成
```

注意：`pnpm test` 当前只输出 `TODO: add tests`，不要误判为已有测试通过。

---

## 7. 协作规则（给 Agent）

### 7.1 代码改动边界

- 可以修改：`src/**`、`docs/**`、配置文件、`docker/**`、`scripts/**`
- 默认不直接修改：`__generated__/api-clients/**`（应通过 `pnpm openapi-ts` 生成）
- 修改路由请优先改 `src/pages/**` 文件结构，不要手写硬编码路由表

### 7.2 编码原则

- KISS：优先局部最小改动，避免引入额外抽象层
- YAGNI：不为“将来可能需要”提前设计复杂扩展点
- DRY：组件/弹窗/组合逻辑出现重复时，抽取到 `components/` 或 `composables/`
- SOLID：
  - 单一职责：页面容器负责编排，复杂逻辑下沉到 composable
  - 依赖倒置：业务逻辑依赖类型接口和 API 抽象，而非硬编码请求细节

### 7.3 样式与交互约束

- 遵循 `docs/DESIGN_SYSTEM.md` 的工业设备场景约束（横屏触控、高对比度、触摸尺寸）
- 优先使用现有设计语言（UnoCSS + 现有组件模式），避免引入新的样式体系

### 7.4 注释与文档语言

- 现有仓库注释以中文为主，新增注释默认使用中文
- 仅在复杂逻辑处写必要注释，避免解释性废话

---

## 8. 常见任务指引

### 新增页面

1. 在 `src/pages/` 下创建对应 `.vue` 文件（必要时创建子目录）
2. 通过文件命名让 auto-routes 自动收敛路由
3. 如果需要指定布局，按现有 `layouts` 约定处理

### 对接新接口

1. 更新 OpenAPI 文档源
2. 执行 `pnpm openapi-ts`（或 `pnpm openapi-ts:fetch`）
3. 在 `src/api/` / 页面模块中消费生成的 query/mutation 能力
4. 不手改 `__generated__/` 文件

### 发布构建

1. `pnpm build`
2. 镜像发布链路参考 `docker/Dockerfile` 与 `docker/entrypoint.sh`
3. 需要 MinIO 产物时使用 `scripts/minio.js`（依赖 `MINIO_ACCESS_KEY` / `MINIO_SECRET_KEY`）

---

## 9. 当前技术债与建议

- 测试体系缺失：建议补齐 Vitest + Vue Test Utils 基线
- `src/router.ts` 中 `handleHotUpdate(router)` 重复调用两次，建议确认是否有意保留
- `README.md` 仍是模板内容，建议后续同步为业务项目说明

---

## 10. 变更记录

| 日期 | 变更内容 | 版本 |
| --- | --- | --- |
| 2026-04-22 | 基于真实仓库结构重写 AGENTS 文档 | v1.0.0 |
