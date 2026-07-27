#!/bin/sh
# Run TokenBuddy from source with hot reload.
#
# Note: the app is tray-first. It starts with NO window — look for the icon in
# the macOS menu bar / Windows system tray. Set TOKENBUDDY_DEBUG_SHOW_WINDOWS=1
# to have the main window appear at startup instead.

. "$(dirname -- "$0")/lib.sh"
ensure_pnpm

echo "启动开发模式。应用是托盘优先的：默认不弹窗口，请看菜单栏图标。"
echo "想直接看到窗口：TOKENBUDDY_DEBUG_SHOW_WINDOWS=1 sh scripts/dev.sh"
cd "$REPO_ROOT"
exec pnpm dev
