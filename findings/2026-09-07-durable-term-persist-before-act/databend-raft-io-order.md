# Raft 中的 IO 执行顺序：内存状态与持久化状态的陷阱

Source: https://segmentfault.com/a/1190000047314653
Author: Databend. Published 2025-10-11. Fetched 2026-09-07 via web_fetch
(raw HTML curl returned 410; this is the fetched article text).

## Invariant (quoted)

Raft 论文要求 "Before responding to RPCs, a server must update its
persistent state"，在实现中需要更精确的表述：**必须等待所有使
`persisted_term >= req.term` 的 IO 完成后，才能返回成功**。

**关键不变式**：log entry (term=T) 在磁盘 → persisted_term ≥ T 也必须在磁盘

## As-is shape (quoted)

收到更高 term 时立即更新内存 `current_term`，异步 save-term。若在
save-term 完成前崩溃，磁盘仍是旧 term，但过程已在新 term 上应答。
