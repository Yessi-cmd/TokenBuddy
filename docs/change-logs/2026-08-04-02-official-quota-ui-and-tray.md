# 2026-08-04 官方额度页面与托盘同步修复

## 目的

修复官方额度已经成功入库、但托盘仍显示 `Unavailable`，以及完整额度页无法在同一视图看到已用、剩余和重置时间、快照被面板裁切的问题。

## 影响文件

- `crates/storage/src/lib.rs`
- `apps/desktop/src/features/quick/QuickSummaryView.tsx`
- `apps/desktop/src/features/quotas/QuotasView.tsx`
- `apps/desktop/src/styles.css`
- `apps/desktop/src/App.test.tsx`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-04-02-official-quota-ui-and-tray.md`

## 行为变化

- QuickSummary 先读取最新活动会话所属账号的额度；该账号没有官方窗口时，回退到最新的 `auth_mode = 'chatgpt'` 官方额度快照。没有任何官方快照时仍显示 `Unavailable`，不会把未知值转成 0。
- 托盘官方额度行新增“刷新”按钮。刷新会调用共享 Core 的官方额度入口，完成后立即重新读取摘要，并展示官方返回的窗口、剩余比例和重置时间。
- 完整额度页改为账号卡片与额度快照列表：官方账号的窗口、已用、剩余、重置、精度和指纹在同一信息层级展示；本地会话推断账号继续明确标注 `Unavailable`。
- 额度页不再继承全局固定 `min-height` 和 `overflow: hidden`，快照会随内容自然增长；宽屏、窄屏和移动宽度分别使用多列、折行和单列布局。
- 页面层级、留白、信息密度和状态反馈按 Apple Design skill 的内容优先、响应式和即时反馈原则收紧，避免使用大面积空白承载主要数据。

## 验证

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo test --workspace --all-targets`：全部通过，包含新增的官方额度跨账号回退测试。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：55 项通过。
- `pnpm --filter @tokenbuddy/desktop build`：通过。
- `sh scripts/build-app.sh`：成功生成 macOS `TokenBuddy.app` 与 DMG；新包已替换到 `/Applications/TokenBuddy.app` 并启动，源码构建产物与安装包二进制 SHA-256 均为 `0368103eaa35dfe0edec1c1c4095cfa55a3f50c8c59df067794401f36263ef8e`。

## 剩余限制

- 官方额度仍依赖 Codex Home 中有效的文件型 OAuth 登录态；TokenBuddy 不在本地刷新或写回凭据。
- 官方额度接口属于随官方客户端验证的后端契约，上游 schema 变化时需要更新独立 parser。
- Windows 真机托盘、网络异常和高 DPI 交互仍需 Windows CI/真机验收。
