# 2026-07-27-06 前端覆盖率测量与补测，并修错误提示被自动刷新抹掉

## 目的

前端此前**没有覆盖率工具**，行覆盖率实测只有 41.92%；`api.ts` 因为在组件测试里被整体 mock，
覆盖率为 0——而它承载的正是"桌面 IPC 与 loopback HTTP 两条通道问同一件事"这个不变量。
把 Rust 与前端合并计算，项目整体覆盖率低于 85%。

## 缺陷：操作错误在用户看到之前就被抹掉

`DashboardView` 只有一个 `error` 槽位。总览的加载副作用在成功时调用 `setError(null)`，而：

- `handleScan` 结束后会 `setRefreshVersion(+1)`，**立即触发**这次加载；
- 另有一个 5 秒定时器同样会触发。

于是"扫描全部来源"里某个来源失败的提示、检测失败、导出失败、本地网页面板启动失败，全都会在
下一次加载成功时被清空——快则几十毫秒。独立扫描每个来源、以便指出**是哪个**来源失败，正是
这套设计的初衷，而这条信息恰恰传不到用户眼前。

修复：把错误拆成 `loadError`（加载副作用独占）与 `actionError`（用户操作独占），渲染时优先
显示后者。加载成功只清自己的槽位，不再擦掉用户刚触发的操作结果。

## 影响文件

- `apps/desktop/package.json`、`pnpm-lock.yaml`：新增 dev 依赖 `@vitest/coverage-v8`
  （新增，非升级）。
- `apps/desktop/vitest.config.ts`：接入 v8 覆盖率，范围限定 `src/**`，排除入口与测试夹具。
- `apps/desktop/src/lib/api.test.ts`（新增）：16 项传输契约测试。
- `apps/desktop/src/App.test.tsx`：新增 18 项面板行为测试。
- `apps/desktop/src/App.tsx`：错误槽位拆分。

## 覆盖的契约

**传输层**：浏览器走 fetch、桌面走 invoke，两者互不触发；失败响应变成异常而非返回空体；
空/`null` 筛选值不进查询串（否则服务端会当成"筛选空字符串"）；`offset=0` 会显式发送；
会话 id 经过 URL 编码；设置在浏览器用 PUT + JSON body、在桌面走 IPC 无 body；四个 rescan
的 snake_case 字段名；浏览器里 `startLocalWebApi` 等自答不发请求。

**面板行为**：会话列表/详情/数据源/Provider/额度各自的数据态与失败态；四个检测互相独立、
一个失败不清空另一个的结果；扫描部分失败时仍统计成功来源并指名失败者；导出在浏览器走 blob
下载、在桌面写盘；设置保存成功与被拒；缺失值渲染为 `Unavailable` 而非 0；托盘面板的额度窗口、
警告、未知态与失败态。

## 验证

- 前端 46 项测试通过；`prettier --check`、`eslint --max-warnings 0`、`tsc -b`、`vite build`
  全部通过。
- 前端行覆盖率 41.92% → **83.44%**（`api.ts` 97.95%，`App.tsx` 80.52%）。
- Rust 侧不受影响：`cargo fmt --check`、`clippy -D warnings`（0）、
  `cargo test --workspace --all-targets`（14 套件）通过，行覆盖率 85.92%。
- 合并计算：(7581 + 358) / (8823 + 429) = **85.8%**。

## 遗留限制

- `App.tsx` 剩余未覆盖部分主要是图表渲染分支与少量格式化边界。
- 前端覆盖率未接入 CI 阈值检查；目前只是可测量，不会自动阻止回退。
