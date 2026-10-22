---
name: sros-config-parameter
description: Add, modify, review, or troubleshoot SROS function parameters in cfg/sros-resources/config/config.csv. Use when a task mentions adding a parameter, changing parameter metadata, Matrix visibility or search, menu placement, vehicle filtering, displayed units, value types, upgrade behavior, or validating a packaged configuration.
---

# SROS 配置参数

把配置参数视为贯穿 CSV、升级数据库、SROS 读取、外部协议和 Matrix 展示的一份接口契约。不要只凭相邻行或截图修改单个字段。

## 开始前

1. 查看当前分支和工作区，保留用户已有修改。
2. 用 `git log -S'<key>' --all -- cfg/sros-resources/config/config.csv` 找参数历史；用户要求参考旧提交时，直接 `git show <commit> -- <files>`，不要凭记忆复刻。
3. 读取 CSV 表头和 `cfg/sros-resources/config/README.md`。使用 CSV 解析器按列名检查数据；行内可能有带逗号的引号字段，禁止用简单字符串分割来判断列位。
4. 同时比较三类样例：同一 `parent_key` 的兄弟参数、同一 `value_type` 的参数、同一业务和单位契约的参数。不得从叉车等专用参数整行照抄。

## 设计参数契约

在编辑前明确并记录以下内容：

- `key`、唯一 `id`、初始 `value`、实际缺省值。
- 代码消费类型与格式，例如单值数值或 `x;y` 复合字符串。
- 存储单位、代码内部单位、SDK/协议单位、产品要求的展示单位，以及每个边界的换算位置。
- 坐标原点、轴正方向、符号含义和允许范围。
- 适用车型、目标菜单路径、是否可编辑、是否支持在线更新、权限和最低版本。

至少用一个非零值走通完整链路。`0` 在米和毫米下都相同，不能证明单位正确。

## 填写 CSV

- `parent_key` 必须存在。沿父链和各层 `order_id` 还原 Matrix 目录编号，确认最终位置；不要只看参数在文件中的物理行号。
- 同一父节点下的 `order_id` 必须唯一。目录项和叶子项的 `is_category` 要与实际树结构一致。
- `vehicle_type=0` 表示通用。其他值必须对照当前车型枚举确认；前端会按车型过滤，误填专用车型会导致参数搜不到。
- `is_valid`、`is_editable`、`is_update_online`、`permission`、`min_version_requirement` 逐项按需求确认，不要从模板继承。
- `value_type` 必须匹配代码读取类型和前端控件。已合入或部署的参数不得直接改 `id`、`key` 或 `value_type`；类型设计错误时按仓库规则废弃旧项并新增参数。尚未合入的新增项应在提交前修正。
- `default_value` 通常应是实际缺省值。部分历史复合字符串把字段名写在该列，Matrix 又会把它展示为“默认值”；遇到此模式必须核对真实前端和升级语义，不能因为有旧样例就照抄。
- 新增参数的 `value` 要能让首次安装得到正确行为。升级会同步除 `value` 外的元数据，修改 `default_value` 或单位不会自动覆盖车辆已有值。

## 单位与 Matrix

`value_units` 不是纯文案。修改它之前必须检查实际使用的 Matrix 页面：

- 经典 `chip-web` 功能参数页会把 `value_units` 直接作为输入框后缀显示。
- 某些专用页面会对 `int`、`float`、`double`、`hex` 做米/毫米换算；`str` 复合值通常不会进入数值换算。
- 对 `x;y` 这类复合 `str`，只把 `value_units` 从 `mm` 改成 `m` 会造成标签和实际值不一致。若要改展示单位，必须同时实现并测试复合值的换算，或同步修改 SROS 解析边界；不能只改 CSV。
- 如果两个坐标需要独立范围、校验或自动换算，优先评估两个数值参数是否更合适；对已经发布的参数遵守不可直接修改 `value_type` 的规则。

单位审查必须沿以下方向双向核对：

```text
Matrix 输入/显示 -> func_config.value -> Settings 读取 -> 业务解析
-> SDK/协议编码 -> SRTOS/设备消费
```

同时更新参数描述、使用说明和示例值。文档单位、界面单位和代码单位必须一致。

## 参数文案与翻译

新增参数，或修改 `config.csv` 中会进入界面的 `name`、`description`、`value_range_text` 后，必须同步更新 `cfg/sros-resources/i18n/config_i18n_*.json`，不要等 CI lint 失败后再补。

- AI 执行自动或手工翻译前，必须读取并纳入目标语言提示词上下文；尤其 `zh-hant` 必须读取 `cfg/sros-resources/i18n/translation_prompt_zh-hant.md`，其他语言读取同目录的 `translation_prompt_<lang>.md`。
- 优先使用仓库脚本自动补全翻译：

  ```bash
  python3 cfg/sros-resources/bin/i18n_auto_translate.py --apply --check --types config
  ```

- 如果已知 MR 目标分支，先 `git fetch origin <target_branch>`，并给上面的命令追加 `--changed-from FETCH_HEAD`，让脚本按目标分支判断新增/变更文案。
- 如果自动翻译缺少 LLM 凭证或校验失败，必须按提示词手工补齐目标语言 JSON，再运行 `--check` 确认无缺失。
- 翻译时保留 key、数字、单位、占位符、Markdown/HTML 标签；`value_range_text` 必须保持分号数量和顺序。

## 验证

先执行仓库现有检查：

```bash
(cd cfg/sros-resources/bin && python3 check_config_scripts.py)
python3 cfg/sros-resources/bin/i18n_auto_translate.py --apply --check --types config
python3 cfg/sros-resources/bin/check_i18n_files_scripts.py --fail-on-missing
python3 cfg/sros-resources/bin/lint_config_parent_key.py
python3 cfg/sros-resources/bin/lint_config_order_id.py
python3 cfg/sros-resources/bin/check_category_without_children.py
```

然后完成与风险相称的验证：

1. 用 CSV 解析器按字段名打印目标行，确认列数、`parent_key`、`vehicle_type`、`order_id`、`value_type`、`value_units` 和编辑属性。
2. 检查 `id`、`key` 唯一性，并运行项目提交钩子要求的校验。
3. 为解析和换算增加非零回归用例；例如验证 `500;200` 在每个边界仍代表同一物理位置。
4. 构建升级包后解包检查包内 `config.csv`，不要只验证源码。
5. 在目标车型升级后核对数据库/API 返回值，以及 Matrix 中的搜索、目录编号、单位后缀、编辑保存和重启持久化。
6. 若修过包，旧包和旧说明不得继续作为交付物；交付目录只保留明确标识的最终版本。

## 评审输出

优先报告会造成以下后果的问题：数值量级错误、参数不可见、升级不生效、目录错误、前端误导或协议单位不一致。每项给出文件/行号、证据、影响和完整修正范围；不要把“改单位文字”描述成已修复端到端单位契约。
