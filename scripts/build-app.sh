#!/bin/sh
# Build the installable application and say where it landed.
#
# `cargo build` alone produces a bare executable under target/, which is not
# something a user can launch on macOS. This produces the real .app / installer.

. "$(dirname -- "$0")/lib.sh"
ensure_pnpm

cd "$REPO_ROOT"
pnpm build

echo
echo "构建产物："
find target/release/bundle -maxdepth 2 -mindepth 1 -print 2>/dev/null || {
    echo "  未找到 bundle 目录，构建可能失败了。" >&2
    exit 1
}
