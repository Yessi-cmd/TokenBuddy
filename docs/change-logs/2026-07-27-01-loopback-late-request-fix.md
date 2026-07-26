# 2026-07-27-01 修复 loopback 服务丢弃迟到请求

## 目的

macOS 上 `local_api_serves_core_data_over_both_loopback_families` 与
`quick_summary_http_p95_stays_within_lightweight_entry_budget` 偶发失败，报
`Connection reset by peer`，重跑即过。排查后确认不是测试不稳定，而是本地网页服务的真实缺陷。

## 根因

`serve` 把两个监听套接字设为非阻塞，以便在一个线程里轮询 IPv4 与 IPv6 两个地址族。在
BSD 语义的系统（macOS）上，`accept` 得到的连接**继承监听套接字的 `O_NONBLOCK`**，于是
`read_request` 里的 `set_read_timeout` 失效：客户端字节尚未到达时 `read` 立刻返回
`WouldBlock`。该错误与"对端已断开"无法区分，连接因此被直接丢弃，客户端看到重置。

Linux 上 `accept` 不继承该标志，所以只在 macOS 显现；本地通常连接与请求字节一起到达，
只有机器负载较高、两者间隔拉开时才触发。真实浏览器面板同样会中招，不只是测试。

## 影响文件

- `apps/desktop/src-tauri/src/web.rs`：`serve` 在 accept 后显式 `set_nonblocking(false)`，
  连接恢复阻塞语义，1 秒读超时重新生效。

## 验证

- 新增回归测试 `a_request_written_after_the_connection_is_established_still_gets_a_response`：
  先建立连接，等待 150ms 再写请求，断言仍返回 200。
- 已确认该测试在**移除修复后必定失败**（响应为空字符串），修复后通过——不是概率性测试。
- `cargo test -p tokenbuddy-desktop`：25 项通过。
- `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test --workspace --all-targets`：通过。

## 遗留限制

- 该服务仍是单线程顺序处理连接，一个慢客户端会阻塞后续请求最多 1 秒（读超时）。本地面板
  场景下可接受，未改动。
