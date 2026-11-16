---
name: simenv-replay-video
description: Use when retained simenv/Gazebo runs need a two-robot replay video, or when asked to visually explain recorded trajectories, waiting/yielding and original task completion (验收回放、俯视视频、怎么通过的).
---

# 验收数据回放视频

把已有记录做成可播放的俯视 MP4：真实轨迹、车头方向、协作建议、实际速度和原任务终态分开显示。支持多个场景；原回执未通过的场景显示“未通过”，不以物理通过或单个 FollowPath 成功代替整轮结果。

## 选记录

- 用户明确要数据回放时直接使用保留记录；要实时录屏时另走实时录制流程。不要把两种产物混称，也不要为离线回放启动仿真、机器人或修改运行参数。
- 从原回执选对应轮次。用户要看“通过过程”时选整体 `passed=true` 的记录；保留并说明已知失败轮次，不能将复测通过写成每次都通过。
- 读取冻结世界与机器人资产。工作目录 SDF 的哈希不同时比较实际墙体／碰撞几何；物理采样插件、换行或 GUI 设置变化不等于几何变化。保留源文件哈希及核对结果。
- 车体轮廓复用原验收校验器的碰撞包络，不能为了好看缩小；图中不画未发生的预测路径。

## 准备

预检 `python3`、Pillow、`ffmpeg`、`ffprobe`、Fontconfig 的 `fc-match` 和 Noto Sans CJK SC 中文字体。数据提取仅用 Python 标准库；缺依赖先报告，不自行安装或改生效环境。

按 [输入格式](references/input-format.md) 写一个本机配置，路径相对该配置解析。不要把本轮目录、镜像、SHA、机器人初始位姿或固定场景集合写进 skill。

```bash
python3 scripts/prepare.py replay-input.json output-replay
python3 scripts/render.py output-replay --preview
```

检查片头、各场景和片尾 PNG：地图／车体比例、目标位置、车头方向、中文实际字形、标注是否遮挡，以及失败／未知状态是否准确。

## 时间与状态

- 原生 CSV 往往含启动和 staging 历史。只使用回执 `motion_start_anchor.sim_ns` 到 `rtf_sim_end` 的窗口；两车必须来自同一连续物理时间轴。丢步、时钟回退、实体更换、非有限数值或窗口不完整直接报错。
- 完成时刻直接用 `terminal_anchors` 的仿真时间。没有锚点就保持未知；不从墙钟、RTF、到达目标附近或 FollowPath 成功猜原任务已完成。
- 协作快照显示为“最近建议”及其记录时刻。速度／转向取物理采样；缺建议记录不是 `NONE`，建议 `WAIT/YIELD` 也不是实际已经停稳或已执行让行。
- 固定倍速并显示仿真时间；绘制抽帧不替代原始连续采样验收，也不产生新的验收结果。

## 生成与核对

```bash
python3 scripts/render.py output-replay --speed 3 --fps 24
```

生成 H.264/yuv420p、1080p、带章节和 faststart 的 `replay.mp4`。先编码到 `.partial.mp4`，核对帧数／尺寸／章节并以 `-xerror` 完整解码后才改正式名称；失败保留中间文件和日志。输出使用新目录，原始 CSV、回执、快照及失败证据只读。

检查编码后的四类画面：等待、让行／转向、原任务完成、片尾判定。确认 `verification.json` 与视频一致，原始数据及资产哈希在 `prepared.json` 可追溯。建议／终态缺记录时，交付说明中明确边界。

交付视频链接、时长、倍速和主要章节，明确它是已有记录的回放。不要用本片宣称新的实车、无线链路或整轮验收通过；没有新跑就不新增通过记录。
