# macOS Core 配置切换测试竞争修复

## 目的

修复 `v0.1.4` 发布前 CI 在 macOS 上暴露的 Core 集成测试时序竞争，避免测试错误地依赖显式刷新与后台 worker 的锁竞争顺序。

## 受影响文件

- `crates/core/tests/core_surface.rs`
- `AI_Coding_Token_Observatory_PROJECT_SPEC.md`
- `docs/change-logs/2026-08-12-02-macos-core-test-race.md`

## 行为变化

- 测试不再断言 `rescan_codex` 本次调用自身必须插入 2 条事件；路径切换会唤醒后台 worker，后台导入可能先完成，使显式刷新合法地报告 0 条新增。
- 测试改为验证稳定契约：`rescan_codex` 返回后 fixture 已导入且事件总数恰好为 2，继续覆盖即时生效与幂等要求。
- 生产代码、导入顺序、cursor、聚合与自启动行为均未改变。

## 验证

- GitHub Actions CI run `31593430292`：Frontend 全部通过；Windows Rust 的 format、Clippy、tests 与 workspace check 全部通过；macOS 仅该竞争断言失败（读取到 0，期望单次报告 2）。
- `git diff --check`：通过。
- 当前本机未安装或未暴露 `cargo`，修复后的 Rust 跨平台测试由下一次 GitHub Actions CI 执行。

## 剩余限制

- 本批次只修复测试对并发先后顺序的错误假设，不改变 `ImportReport` 表示“本次调用新增量”的语义。
- `v0.1.4` 标签仅在后续 Windows/macOS CI 全部通过后创建。
