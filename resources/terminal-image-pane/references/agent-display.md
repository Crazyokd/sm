# 代理打开图片与验证

## 定位目标

先列出必要元数据，在标题和目录匹配的候选中用近期消息确认对话。不要依赖工具继承的 pane ID，也不要把活动窗口当成对话身份。

```bash
tmux list-clients -F '#{client_tty} session=#{session_name} window=#{window_id}'
tmux list-panes -a -F '#{pane_id} #{window_id} #{pane_current_path} #{pane_title}'
tmux capture-pane -p -S -60 -t "$candidate_pane"
```

将唯一匹配的 pane ID 赋给 `target_pane`，将已检查图片的绝对路径赋给 `image_path`。只有候选不明确且无法继续收窄时才询问。不要在 skill 中存储某次运行的 pane ID、窗口 ID、图片临时路径或截图。

## 调用

从该 pane 所属的 tmux server/session 获取环境；工具 shell 中没有 `TMUX` 或 `DISPLAY` 并不代表用户终端不支持图片：

```bash
tmux_socket=$(tmux display-message -p -t "$target_pane" '#{socket_path}')
tmux_server_pid=$(tmux display-message -p -t "$target_pane" '#{pid}')
tmux_session=$(tmux display-message -p -t "$target_pane" '#{session_id}')
tmux_context="$tmux_socket,$tmux_server_pid,${tmux_session#\$}"
tmux show-environment -t "$tmux_session" DISPLAY
tmux show-environment -t "$tmux_session" XAUTHORITY
```

`show-environment` 中未设置 `XAUTHORITY` 本身不表示异常。若 `DISPLAY` 缺失或陈旧，从真实桌面/终端进程核对；不要猜一个显示号或修改整个 server 的环境。辅助程序创建的窗格继承 tmux session 的环境。环境正确时可以直接调用：

```bash
# viewer 为已确认支持分栏的 term-image，或包内脚本的绝对路径。
TMUX="$tmux_context" TMUX_PANE="$target_pane" "$viewer" --right "$image_path"
```

环境需要显式传递，或需要原子获取新 pane ID 时，用 Python 参数列表构造 tmux 命令，避免把图片文件名当 shell 代码：

```python
import shlex
import subprocess

# viewer / image_path / target_pane / display / xauthority 来自核实结果。
# xauthority 未设置时使用 None。
command = ["env", f"DISPLAY={display}"]
if xauthority:
    command.append(f"XAUTHORITY={xauthority}")
command += ["python3", viewer, image_path]
pane = subprocess.check_output([
    "tmux", "split-window", "-h", "-p", "45", "-d", "-P", "-F", "#{pane_id}",
    "-t", target_pane, shlex.join(command),
], text=True).strip()
```

下方分栏将 `-h` 改为 `-v`。第二张图需要在已有图片区下方时，将目标改为本次已记录的图片 pane。不要使用 `send-keys` 向聊天程序注入命令，不要强行切到无关窗口。

## 核验与清理

确定真实终端的 X11 窗口后，可用现有的截图工具捕获该窗口，再通过图像查看工具检查；例如 `import -display "$display" -window "$terminal_window" "$screenshot_path"`。终端窗口 ID 应从对应桌面实时发现，不能复用旧 ID。

只有核对过实际画面，才能报告“已在终端展示”。检查两张图同时可见、布局边界正确、聊天仍可输入。没有截图能力时明确“已启动预览，画面未核验”，不要反复创建预览碰运气。

修改渲染辅助程序或升级 ueberzug++ 时，再验证调整窗格、切换 tmux 窗口和关闭一张图后其余图片仍正常。切换窗口要先记录原窗口并在验证后恢复；常规展示不必主动切换用户窗口。重新发现 X11 子窗口，调整布局可能会重建它们。

退出优先在对应图片 pane 按 Enter。程序化清理只针对已记录的自有 pane；不要广泛终止 ueberzug++，也不要取消其他用户 hook。SIGKILL 无法保证退出清理。

## hook 回归

包内脚本针对 ueberzug++ 2.9.10 的四个 tmux hook 做索引隔离。所有槽使用全局范围，避免局部 hook 数组遮蔽用户继承的全局 hook；清理不依赖原 pane 仍然存在。其他用户自行设置的局部同名 hook 仍可能遮蔽这些全局回调，遇到该情况先检查覆盖关系，不能删除用户配置。

在 skill 目录下运行：

```bash
python3 scripts/test_term_image.py
```

三个测试使用独立的临时 tmux socket，验证已有/并行预览 hook 保留、继承 hook 实际执行、目标 pane 消失后仍能清理。测试不连接用户的 tmux server，也不证明桌面可见性。
