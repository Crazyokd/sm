#!/usr/bin/env python3
"""Preview an image or GIF in a local X11 Alacritty window or tmux pane."""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile


SESSION_HOOKS = ("session-window-changed", "client-detached", "window-layout-changed")
GLOBAL_HOOKS = ("client-session-changed",)


@contextmanager
def renderer_environment():
    environment = os.environ.copy()
    if not environment.get("TMUX"):
        yield environment
        return
    tmux = shutil.which("tmux")
    if tmux is None:
        raise RuntimeError("tmux executable not found")
    index = 1000000000 + os.getpid()
    hooks = SESSION_HOOKS + GLOBAL_HOOKS
    # Normalize only ueberzug++ 2.9.10 hook calls; revisit when its tmux integration changes.
    with tempfile.TemporaryDirectory(prefix="term-image-") as directory:
        shim = Path(directory) / "tmux"
        shim.write_text(
            f"#!{sys.executable}\n"
            "import os, sys\n"
            "args = sys.argv[1:]\n"
            "if 'set-hook' in args:\n"
            "    pos = args.index('set-hook')\n"
            "    tail = args[pos + 1:]\n"
            f"    hook_pos = next((i for i, a in enumerate(tail) if a in {hooks!r}), None)\n"
            "    if hook_pos is not None:\n"
            "        remove = any(a.startswith('-') and 'u' in a for a in tail[:hook_pos])\n"
            f"        args = [*args[:pos], 'set-hook', '-gu' if remove else '-g', tail[hook_pos] + '[{index}]', *tail[hook_pos + 1:]]\n"
            f"os.execv({tmux!r}, [{tmux!r}, *args])\n"
        )
        shim.chmod(0o700)
        # ueberzug++ owns only this process's numeric hook slots.
        environment["PATH"] = directory + os.pathsep + environment.get("PATH", os.defpath)
        try:
            yield environment
        finally:
            for hook in hooks:
                subprocess.run([tmux, "set-hook", "-gu", f"{hook}[{index}]"],
                               env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    placement = parser.add_mutually_exclusive_group()
    placement.add_argument("--right", action="store_true", help="open a new tmux pane on the right")
    placement.add_argument("--below", action="store_true", help="open a new tmux pane below")
    args = parser.parse_args()
    image = args.image.resolve()
    if not image.is_file():
        parser.error(f"not a file: {image}")
    if args.right or args.below:
        pane = os.environ.get("TMUX_PANE")
        if not os.environ.get("TMUX") or not pane:
            parser.error("--right and --below require tmux")
        command = shlex.join([sys.executable, str(Path(__file__).resolve()), str(image)])
        return subprocess.run(
            ["tmux", "split-window", "-h" if args.right else "-v", "-p", "45", "-d", "-t", pane, command]
        ).returncode
    if not os.environ.get("DISPLAY"):
        parser.error("requires the local X11/XWayland desktop (DISPLAY)")
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        parser.error("run this command in an interactive terminal")

    def interrupt(signum, frame):
        raise KeyboardInterrupt

    for sig in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, interrupt)

    with renderer_environment() as environment:
        renderer = subprocess.Popen(
            [str(Path.home() / ".local/bin/ueberzugpp"), "layer", "--output", "x11", "--no-cache"],
            stdin=subprocess.PIPE,
            text=True,
            env=environment,
        )
        try:
            print("\033[?1049h\033[2J\033[HEnter / Ctrl+C: close preview", end="", flush=True)
            size = None
            while renderer.poll() is None:
                current_size = os.get_terminal_size(sys.stdout.fileno())
                if current_size != size:
                    columns, rows = size = current_size
                    renderer.stdin.write(json.dumps({
                        "action": "add", "identifier": "preview", "path": str(image),
                        "x": 0, "y": 1, "max_width": columns, "max_height": max(1, rows - 2),
                    }) + "\n")
                    renderer.stdin.flush()
                if select.select([sys.stdin], [], [], 0.2)[0]:
                    sys.stdin.readline()
                    return 0
            return renderer.returncode
        except KeyboardInterrupt:
            return 0
        finally:
            try:
                for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
                    signal.signal(sig, signal.SIG_IGN)
                try:
                    renderer.stdin.close()
                except BrokenPipeError:
                    pass
                try:
                    renderer.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    renderer.terminate()
                    try:
                        renderer.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        renderer.kill()
                        renderer.wait()
            finally:
                print("\033[?1049l", end="", flush=True)



if __name__ == "__main__":
    sys.exit(main())
