#!/bin/sh
# Launch the locally built application.
#
# Without arguments the app starts tray-only, exactly as a user would see it.
# Pass --window to force the main window open (debug builds honour this too).

. "$(dirname -- "$0")/lib.sh"

app="$REPO_ROOT/target/release/bundle/macos/TokenBuddy.app"
binary="$REPO_ROOT/target/release/tokenbuddy-desktop"

if [ "${1:-}" = "--window" ]; then
    TOKENBUDDY_DEBUG_SHOW_WINDOWS=1
    export TOKENBUDDY_DEBUG_SHOW_WINDOWS
fi

if [ -d "$app" ]; then
    echo "启动 $app"
    echo "应用是托盘优先的：请看菜单栏右侧的图标，不会自动弹窗口。"
    exec open -a "$app"
elif [ -x "$binary" ]; then
    echo "未找到 .app 包，直接运行可执行文件 $binary"
    exec "$binary"
else
    echo "还没有构建产物。先执行：sh scripts/build-app.sh" >&2
    exit 1
fi
