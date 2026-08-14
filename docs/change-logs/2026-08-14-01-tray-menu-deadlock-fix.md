# 2026-08-14-01 Windows 托盘菜单打开窗口死锁修复（v0.1.8）

## 目的

修复 Windows 真机反馈：托盘图标右键菜单中的「打开完整面板」（以及左键/双击等窗口创建路径）无法打开窗口，且触发后整个应用主线程死锁（所有窗口消息循环停止响应、CPU 归零）。

## 根因

Windows 上托盘上下文菜单在主线程序列里运行自己的模态循环（`TrackPopupMenu`）。v0.1.7 之前的实现把 WebView2 窗口的**创建 + show** 直接放在菜单项回调里同步执行；WebView2 环境初始化需要消息泵配合，而菜单模态循环不提供，导致主线程序死锁（在 Windows Server/VM 环境稳定复现：死锁实例的全部窗口 `SendMessageTimeout` 超时、CPU 0%）。单实例转发的 `show_window` 走普通消息分发，因此同一代码路径在非菜单上下文下正常。

## 修复

- `apps/desktop/src-tauri/src/lib.rs`：新增 `deferred_on_main`，把窗口创建/显示推迟到下一个主循环迭代（`thread::spawn` + `run_on_main_thread`），脱离菜单模态上下文；托盘菜单（打开快速摘要 / 打开完整面板）、托盘左键单击/双击、单实例转发统一改走该路径。
- 新增环境变量门控的 `debug_log`（`TOKENBUDDY_LOG_FILE`），窗口构建失败时写入文件，避免托盘应用无控制台导致故障不可诊断。

## 行为变化

- 菜单点击后窗口在下一轮主循环出现（延迟为毫秒级，远小于 WebView2 启动时间），感知无差异。
- 其余行为不变。

## 验证（本机 Windows 真机，MSVC debug 构建 + UI 自动化）

- 托盘右键菜单「打开完整面板」：主窗口 2 秒内可见且消息循环响应正常（PASS）。
- 托盘右键菜单「打开快速摘要」：快速面板窗口可见（PASS）。
- 第二实例转发打开主面板：PASS；全程应用保持响应。
- `cargo fmt --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`：通过。
- `cargo test --workspace --all-targets`（MSVC，含 desktop MockRuntime 测试 34 项）：全部通过。
- 前端无改动。

## 剩余限制

- Windows 托盘其余真机场景（自启动、高 DPI、隐藏窗口持续采集）仍归入未完成的 `Phase 4b 跨平台真机交互补验`。
