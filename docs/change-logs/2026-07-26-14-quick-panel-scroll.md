# Quick panel 底部滚动与会话语义

## 目的

解释并避免 `Fixture session` 被误认为总计，同时修复 quick panel 内容超过固定窗口高度时底部被裁切的问题。

## 受影响文件

- `apps/desktop/src/styles.css`

## 行为变化

- `Fixture session` 明确属于历史脱敏测试会话；“今日 Token / 本地事件汇总”才是所有已导入事件的总计。
- quick panel 外层现在拥有独立的纵向滚动区域和底部安全留白，内容超出窗口时可以滚动查看完整指标、官方额度和提示，不再被宿主窗口硬切断。
- 保留 macOS 风格的卡片、箭头和半透明材质；滚动条使用细、低干扰样式。

## 验证

- `pnpm --filter @tokenbuddy/desktop test`：7 项通过。
- `pnpm --filter @tokenbuddy/desktop format:check`：通过。
- `pnpm --filter @tokenbuddy/desktop build`：通过。
- `pnpm --filter @tokenbuddy/desktop tauri build --debug`：通过，重新生成 debug `.app` 和 `.dmg`。

## 剩余限制

- 既有本地 `Fixture session` 数据未自动删除；如需清理，应单独确认删除范围。
- 本轮 Computer Use 仍未完成桌面截图复验，需在用户桌面解锁后确认滚动到底部的最终视觉效果。
