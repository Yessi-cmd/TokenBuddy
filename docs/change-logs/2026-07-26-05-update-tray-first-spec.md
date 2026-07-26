# Update Tray-first runtime requirements

## Purpose

将最新的 Tray-first（托盘优先）运行方案和 MVP 调整纳入项目指导文档，明确后台采集 Core、菜单栏 / 系统托盘、`QuickSummary`、共享桌面与网页入口，以及本地服务的 loopback 安全边界。

## Affected files

- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-07-26-05-update-tray-first-spec.md`

## Behaviour changes

- 文档现在要求应用启动后默认运行单实例后台 Core，不自动弹出完整桌面面板。
- macOS 菜单栏、Windows 系统托盘、轻量弹窗、完整桌面面板和本地网页面板必须共享 Core、SQLite、统计语义和查询服务。
- 新增 `QuickSummary` 契约和轻量弹窗的资源边界，禁止轻量入口扫描原始日志或执行复杂聚合。
- 明确完整桌面面板与本地网页面板共用 React SPA；本地网页服务只能按需绑定 `127.0.0.1` 和 `::1`。
- MVP 清单、Phase 4 实施顺序、初始任务后的补充要求和实施状态已同步更新。

## Verification performed

- `git diff --check`
- 使用 `rg` 检查 Tray-first、`QuickSummary`、loopback 绑定和 Phase 4 状态均已写入规格。
- 本批仅修改文档，未重新运行代码测试；新增要求对应的实现和真机性能验证仍待 Phase 4b。

## Remaining limitations

- 当前代码尚未实现单实例后台常驻 Core、macOS 菜单栏、Windows 系统托盘、`QuickSummary`、轻量 Popover、本地 loopback Web API 或共享桌面 / 网页入口。
- 当前实现仍保留显式扫描和按需打开初始桌面面板；这些行为不代表新的默认运行方式。
- Claude Session、OTel、CC Switch、Cockpit、导出和 Provider 官方额度数据源仍未完成；本地代理继续保持在 MVP 之外。
