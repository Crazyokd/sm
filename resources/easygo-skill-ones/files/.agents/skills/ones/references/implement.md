# 实现

当用户要实现 ONES 里的需求时，用这个流程。

## 进入条件

1. 先读 [common.md](./common.md)
2. 获取 task info
3. 编码前先确认当前状态符合入口要求

`SROS开发` 需求流常用状态：

- `需求导入`：`5WJAdd7o`
- `开发中`：`K7mLx1ej`
- `产品确认`：`3GxUDUn8`

## 流程

1. 校验当前状态
2. 需要时通过 transition API 把工单推进到 `开发中`，transit 后立即用 `tasks/update` 把 `assign` 回写为 ONES token 用户 UUID（详见 [common.md](./common.md) 「默认负责人 / 问题解决人」），防止工作流副作用把解决人换走
3. 提取需求意图、验收标准、约束、模块和关联项
4. 探索代码库并确定改动范围
5. 改动较大时，在 commentary 里先写一版简短实现计划
6. 增量实现
7. 涉及代码改动时补测试
8. 需要时在 CMake 中注册新增源文件或测试文件
9. 编译和测试
10. 提交 commit，并创建带 ONES ID 的 MR
11. 按需要回写 ONES，并流转到 `产品确认`

## 实现说明

- 如果 ONES 描述里已经有 AI 分析报告，优先复用已确认的根因和关键代码路径
- 纯配置修改可以不补代码测试脚手架，但仍应执行相应的生成或校验命令
- 代码改动应覆盖核心路径、边界场景和集成点

## 提交标题

```text
#<ONES_ID> feat(<scope>): <summary>
```

## ONES 完成回写

用户要求在实现完成后回写 ONES 时：

1. 加一条简短完成评论，并附上 MR 链接
2. 把状态从 `开发中` 流转到 `产品确认`
3. 在同一次 `tasks/update` 中：
   - `assign` 回写为 ONES token 用户 UUID（避免 transit 副作用把解决人换走，统计才能归属）
   - 测试工程师字段 `Q69TCt3s` 为 `R2bkoeif`
   - 软件解决版本号字段 `PL9oJAdQ` 为 `待确认`
