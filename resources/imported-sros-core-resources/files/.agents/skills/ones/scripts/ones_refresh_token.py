#!/usr/bin/env python3
"""刷新 ~/.ones_token：通过持久化 Chromium profile 提取登录 cookie。

SR ONES 实例走飞书 OAuth 扫码登录，登录后 `.ones.standard-robots.com` 域下
保留两枚 cookie：

  ones-uid → ~/.ones_token 的 uuid
  ones-lt  → ~/.ones_token 的 token

本脚本拉起 Playwright 自带的 Chromium，复用 `~/.cache/ones-chromium-profile`
作为持久化 profile（飞书会话还在时无需重新扫码），等到两枚 cookie 出现就
写回 `~/.ones_token`（权限 600）。

用法：

  ones_refresh_token.py          # 默认有界面，会弹窗等扫码
  HEADLESS=1 ones_refresh_token.py  # 假设会话仍有效，无界面静默刷新
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import time
from pathlib import Path

ONES_URL = 'https://ones.standard-robots.com:10120/project/'
TOKEN_FILE = Path.home() / '.ones_token'
PROFILE_DIR = Path.home() / '.cache/ones-chromium-profile'
TIMEOUT_S = 300


def _find_chromium() -> Path:
    cache = Path.home() / '.cache/ms-playwright'
    candidates = sorted(cache.glob('chromium-*/chrome-linux64/chrome'), reverse=True)
    if not candidates:
        sys.exit(
            'Playwright Chromium 未安装；请先在能联网的环境执行：python3 -m playwright install chromium'
        )
    return candidates[0]


def _find_pair(cookies: list[dict]) -> tuple[str | None, str | None]:
    by_name = {c['name']: c['value'] for c in cookies}
    return by_name.get('ones-uid'), by_name.get('ones-lt')


def _atomic_write_token(uid: str, tok: str) -> None:
    """以 0600 权限原子写入 TOKEN_FILE，避免短暂的宽权限窗口。"""
    parent = TOKEN_FILE.parent
    parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_path = tempfile.mkstemp(prefix='.ones_token.', dir=parent)
    try:
        with os.fdopen(fd, 'w') as f:
            json.dump({'uuid': uid, 'token': tok}, f, ensure_ascii=False)
        # mkstemp 默认就是 0600，这里显式再 chmod 一次保险
        os.chmod(tmp_path, 0o600)
        os.replace(tmp_path, TOKEN_FILE)
    except Exception:
        try:
            os.unlink(tmp_path)
        except FileNotFoundError:
            pass
        raise


def main() -> None:
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        sys.exit(
            '依赖缺失：pip3 install --user playwright && python3 -m playwright install chromium'
        )

    PROFILE_DIR.mkdir(parents=True, exist_ok=True)
    headless = os.environ.get('HEADLESS') == '1'
    chrome_bin = _find_chromium()

    with sync_playwright() as p:
        ctx = p.chromium.launch_persistent_context(
            user_data_dir=str(PROFILE_DIR),
            executable_path=str(chrome_bin),
            headless=headless,
            viewport={'width': 1280, 'height': 900},
            args=['--no-first-run', '--no-default-browser-check'],
        )
        page = ctx.pages[0] if ctx.pages else ctx.new_page()
        page.goto(ONES_URL, wait_until='domcontentloaded')

        uid = tok = None
        deadline = time.time() + TIMEOUT_S
        while time.time() < deadline:
            # 只取当前 ONES URL 适用的 cookie，避免 endswith 匹中类似域名（如 evilones.standard-robots.com）
            uid, tok = _find_pair(ctx.cookies(ONES_URL))
            if uid and tok:
                break
            time.sleep(2)

        ctx.close()

    if not (uid and tok):
        sys.exit('超时：5 分钟内未拿到 ones-uid / ones-lt cookie。请确认已完成飞书扫码登录。')

    _atomic_write_token(uid, tok)
    print(f'已写入 {TOKEN_FILE}：uuid={uid} token=...{tok[-6:]}')


if __name__ == '__main__':
    main()
