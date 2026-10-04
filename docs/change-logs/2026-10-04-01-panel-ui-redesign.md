# 2026-10-04-01 面板 UI 重构：交互修正、布局调整与动效

## 目的

重构完整面板与网页面板的 UI：修复不合理的交互与布局位置，并加入现代化、科技感的动效。数据契约、Core/Tauri/Web API 均未改动。

## 修复的交互 / 布局问题

- **总览页路径输入有隐藏副作用**：总览页的路径输入框在“扫描全部来源”时会把路径传给 `rescan_*`，Core 会**静默持久化**到设置；而“检测”按钮检测的是未保存的输入。现移除总览页路径输入；扫描一律使用设置中保存的路径（传 `null`）。路径编辑与检测迁到设置页（检测当前输入、保存前可验证），数据源页提供按已保存路径的“检测 / 全部检测”。
- **筛选面板占据首屏**：9 个筛选字段始终展开，压在指标卡之上。改为紧凑范围栏：快捷时间段（今天 / 近 7 天 / 近 30 天 / 全部）、日期区间、应用、搜索、“更多筛选”（带激活数，可折叠，`inert` 防止折叠时被 Tab 聚焦）；“清除筛选”只在有筛选时出现；导出合并为分段按钮。
- **加载中显示为 Unavailable**：首次加载前指标显示“Unavailable”，语义错误（是“还没读到”而非“缺失”）。现显示骨架屏；会话、Provider、数据源、会话详情同样区分加载中与空。
- **导航无当前页标识**：新增 `aria-current` 与滑动高亮指示器；`/sessions/:id` 归属“会话”。
- **操作结果覆盖状态胶囊**：扫描/导出/打开网页的结果原先写进工具栏状态胶囊（被截断，且盖住数据层状态）。改为右下角 Toast；状态胶囊只表示数据层连接状态；操作错误为可关闭的提示条。
- **浏览器内显示“本地网页”按钮**：已在网页面板中时不再显示。
- **设置页**：OTLP 端口逐键校验导致无法输入（输入“8”即被清空，打不出 8080），改为保存时解析并给出内联提示；设置读取失败时“保存设置”会把空默认值写回，现禁用保存；新增未保存修改提示、放弃修改、Ctrl/⌘+S、底部吸附保存栏；开关统一为 toggle 样式。
- **额度页**：“官方订阅渠道”区块（工作区未提交改动）原为无样式裸 HTML，现为渠道卡片 + 可折叠接入说明（命令可复制）；刷新按钮移至顶栏；各窗口以进度条展示已用百分比（credits 窗口不画条，避免伪装成 0%）。
- **会话页**：新增搜索（250ms 防抖）与“加载更多”分页；总览会话列表限高滚动，点击已选会话可取消选择，详情面板吸顶。
- 会话详情新增 Token 构成条（新输入 / 缓存读取 / 输出），仅当三项均已知时绘制；模型表增加相对用量条。

## 动效

网格 + 极光背景缓动、页面分段入场、卡片错峰浮现、指标数字滚动（首个已知值从 0 计起；`null` 不插值）、鼠标跟随光晕、主按钮扫光、扫描进度线、状态点脉冲、空状态雷达、进度条填充与流光、Toast 计时条；托盘弹窗行入场（仅 translate/opacity，不影响窗口高度测量）与数值闪烁。`prefers-reduced-motion: reduce` 下全部动效关闭；无 `matchMedia`（测试环境）时数字不做动画。

## 影响文件

- 新增：`apps/desktop/src/lib/motion.ts`、`apps/desktop/src/lib/toast.ts`、`apps/desktop/src/components/Toaster.tsx`
- 修改：`App.tsx`、`styles.css`（面板样式重写，托盘样式保留并追加动效）、`components/Navigation.tsx`、`components/Presentation.tsx`、`lib/filters.ts`、`features/{dashboard,sessions,providers,sources,quotas,settings,quick}/*`、`App.test.tsx`
- 保留了工作区内已有的未提交改动（`startPolling` 轮询、额度渠道内容、`subscription_session` 标签）。

## 验证

- `pnpm --filter @tokenbuddy/desktop exec prettier --check .`：通过
- `pnpm --filter @tokenbuddy/desktop exec eslint . --max-warnings 0`：通过
- `tsc -b` / `pnpm build:web`：通过
- `pnpm --filter @tokenbuddy/desktop exec vitest run`：3 个文件、67 项全部通过（替换总览检测相关用例；新增：总览无路径编辑、设置页检测输入路径、数据源页检测、扫描传 `null`、导航当前页、浏览器隐藏“本地网页”、端口逐键输入、读取失败禁止保存）
- 视觉检查：使用临时 Vite 配置和脱敏模拟 `/api`，在 Chrome 中逐页截图检查（总览、筛选展开、会话详情、额度、设置脏状态与检测、数据源、托盘弹窗）。临时配置已删除。
- 本机未安装 Rust 工具链，`cargo fmt/clippy/test/check` 未能运行；本批次未改动任何 Rust 代码。

## 剩余限制

- 托盘弹窗仅加入动效与额度进度条，原生菜单风格未改。
- 未在 Tauri 真机窗口（WKWebView / WebView2）中目视验证；`backdrop-filter` 与 `color-mix()` 在旧版 WebView2 上会退化为不透明背景 / 无发光。
- 指标数字滚动依赖 `requestAnimationFrame`，隐藏窗口中不运行，下一次可见时直接落到最终值。
