# 开机自启设置选项

## 目的

在设置页提供清晰、可发现的“开机自启”选项，让用户能够显式决定是否在登录系统后于后台启动 TokenBuddy，同时消除原有复选框“立即生效”与实际需要保存设置之间的文案矛盾，并将该批次作为 `v0.1.4` 发布。

## 受影响文件

- `apps/desktop/src/features/settings/SettingsView.tsx`
- `apps/desktop/src/styles.css`
- `apps/desktop/src/App.test.tsx`
- `apps/desktop/package.json`
- `apps/desktop/src-tauri/tauri.conf.json`
- `Cargo.toml`
- `Cargo.lock`
- `.github/workflows/release.yml`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-12-01-startup-option.md`

## 行为变化

- 设置页新增独立的“启动行为 / 后台运行”区块和“开机自启”开关。
- 开关说明登录后只启动后台采集与托盘入口，不自动弹出完整面板。
- 选项默认关闭，用户修改后须点击“保存设置”才会持久化并同步系统自启动状态。
- 切换开关时页面明确提示“尚未保存”，并清除先前的保存错误提示。
- 继续复用现有跨平台 `auto_start` 契约：Windows 使用当前用户 `Run` 注册表项，macOS 使用 LaunchAgent。
- 桌面端、Tauri、Rust workspace 与锁文件版本统一升级到 `0.1.4`；Release 说明同步突出本批次功能。

## 验证

- `pnpm exec vitest run src/App.test.tsx -t "offers a persisted option"`（在 `apps/desktop` 中执行）：1 passed，38 skipped。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop lint`：通过。
- `pnpm --filter @tokenbuddy/desktop test`：56 passed。
- `pnpm --filter @tokenbuddy/desktop build`：TypeScript 与 Vite 生产构建通过。
- `git diff --check`：通过。
- Rust 格式、Clippy、测试与 Tauri 构建未执行：当前 Windows 环境未安装或未暴露 `cargo`；本批次未修改 Rust、Tauri 配置或依赖。

## 剩余限制

- 尚未在本机执行 Windows 注销/重新登录后的真实自启动验收，也未执行 macOS LaunchAgent 真机验收；规格中的 `Phase 4b 跨平台真机交互补验` 保持未完成。
- 本批次未新增安装器首装引导，自启动仍由设置页中的用户显式选择控制。
