#!/bin/sh
# Shared helpers for the scripts in this directory.
#
# The repo's package.json scripts delegate with `pnpm --filter`, so pnpm has to
# be reachable as a plain `pnpm` command — not only through `corepack pnpm`.
# On a machine where Node was installed without enabling corepack's shims that
# is not true, and the build fails deep inside a nested script with a bare
# "pnpm: command not found". Resolve it once, here.

set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export REPO_ROOT

ensure_pnpm() {
    if command -v pnpm >/dev/null 2>&1; then
        return 0
    fi

    if ! command -v corepack >/dev/null 2>&1; then
        echo "找不到 pnpm，也找不到 corepack。" >&2
        echo "请安装 Node.js 24+（自带 corepack），或直接安装 pnpm 11+。" >&2
        exit 1
    fi

    # Put a shim on PATH so nested `pnpm --filter ...` calls resolve too.
    shim_dir="$REPO_ROOT/target/.script-bin"
    mkdir -p "$shim_dir"
    printf '#!/bin/sh\nexec corepack pnpm "$@"\n' > "$shim_dir/pnpm"
    chmod +x "$shim_dir/pnpm"
    PATH="$shim_dir:$PATH"
    export PATH

    echo "提示：pnpm 不在 PATH 中，本次通过 corepack 代理。" >&2
    echo "      想一劳永逸，请执行一次：corepack enable" >&2
}
