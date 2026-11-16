# 输入格式

`prepare.py` 面向原生 `StepPhysics` CSV、`pair_run` 形状的回执和 Nav2 协作 JSONL；其他记录先导出成这一严格格式，不猜字段或时钟。两台车的原生 model 名是 `robot1`、`robot2`，终态锚点键是 `1`、`2`。

配置示例（所有路径相对配置文件；例中轮廓仅作格式示例，必须换成该车经过核对的碰撞包络）：

```json
{
  "source_label": "本次验收来源标识",
  "world": {"sdf": "assets/frozen-world.sdf", "wall_model": "walls"},
  "body": {
    "source": "assets/frozen-robot.sdf",
    "outline": [[-0.4,-0.2],[0.4,-0.2],[0.4,0.2],[-0.4,0.2]]
  },
  "scenes": [{
    "title": "场景名称 · 冲突阶段",
    "result": "case/result.json",
    "physics": ["case/robot1/physics-run.csv", "case/robot2/physics-run.csv"],
    "cooperation": ["case/robot1-cooperation.jsonl", null],
    "goals": [{"x": 1.0, "y": 0.0}, {"x": -1.0, "y": 0.0}]
  }]
}
```

- `source_label` 标明该轮来源。任意数量的场景按输入顺序播放；封面最多列四个，章节包含全部场景。
- `world.wall_model` 精确选墙体模型，避免把地面、装饰或机器人当障碍。辅助工具支持该模型下 link/collision 的 box 与平面 yaw；倾斜、外部相对坐标或非 box 墙体需先明确导出，不作近似。
- `body.outline` 是以机器人基准 link 为原点、米为单位、+X 为车头的碰撞包络。`body.source` 记录原模型资产哈希；轮廓与该模型的一致性仍需核对。轮廓包含所有原验收使用的碰撞部件，不用图标尺寸代替。
- `goals` 是该次原任务的世界坐标，不是未校验的 map 坐标。若来自地图站点，复用该轮记录的坐标变换后再填入。
- 回执必须有布尔 `passed`、`motion_start_anchor.sim_ns`、`rtf_sim_end`。`terminal_anchors`、`physical`、`real_time_factor` 可以缺失；缺失项显示未知或未记录。窗口无法界定时先补导出，不能猜结束时间。
- 完成锚点示例：`"terminal_anchors":{"1":{"sim_ns":123},"2":{"sim_ns":456}}`；这绑定原业务任务完成，不绑定临时段结果。
- 原生 CSV header 为 `schema,iteration,sim_ns,dt_ns,model,link_entity,valid,px,py,pz,qx,qy,qz,qw,vx,vy,vz,wx,wy,wz,error`。schema 必须为 `1`；窗口内每次物理更新都在，位姿为世界系米、四元数；工具检查两车相同步点。
- 协作 JSONL 使用 `kind=cooperation`、`sim_ns`、`state` 和 `components[].recommendation`；其他 kind 不用于建议。状态 `1` 表示模型可用；建议 `0/1/2/3` 为 NONE/CONTINUE/WAIT/YIELD。日志缺失时配置为 `null`，不能填空的假 NONE 事件。
- 物理指标直接来自原回执 `physical.single_vehicle`、`minimum_pair_continuous_bound_m`。视频不会重算或替原回执判定门禁，亦不固定某个项目的 20 mm、3 ms 或实时率阈值。

输出：`prepared.json` 保存规范化轨迹、建议、终态和所有来源哈希；预览 PNG；`replay.mp4`；`verification.json` 记录编码／完整解码检查、倍速、章节和视频哈希；`encode.log` 保留编码诊断。
