# 修复

当用户要从 ONES 缺陷或现场问题走到代码修复和 MR 时，用这个流程。

## 流程

1. 执行或复用 [analyze.md](./analyze.md) 的分析流程
2. 如果用户要求回写 ONES，尽早上传分析结果
3. 做最小且站得住脚的修复
4. 补回归测试和边界保护
5. 编译并运行相关测试
6. 提交 commit，并创建带 ONES ID 的 MR
7. 如果回写 ONES 时触发了 `transit`，紧跟一次 `tasks/update` 把 `assign` 与 `HiRJZNR7`（问题解决人）回写为 ONES token 用户 UUID（详见 [common.md](./common.md) 「默认负责人 / 问题解决人」），避免统计丢失

## 修复原则

- 优先最小化改动
- 先修根因，再在必要位置补防御
- 保持周围代码风格和现有模式

## 测试要求

至少覆盖：

1. 精确复现回归触发条件
2. 边界值
3. 一个正常路径，证明没有引入额外破坏

## 提交标题

```text
#<ONES_ID> fix(<scope>): <summary>
```

## MR 描述

让 reviewer 快速看懂：

1. Summary：ONES 链接和影响范围
2. 根因：修改前后的逻辑差异
3. Changes：具体改了什么
4. 测试计划：验证计划
