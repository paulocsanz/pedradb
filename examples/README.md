# PedraDB by example

A ladder, not a dump. Each program is a complete binary: copy one file, or run
it from this repo. The kernel examples use only `pedradb-core`. The last three
are layers on the same directory (`pedradb-ops`, `rocksdb-compat`).

```sh
cargo run -p pedradb-examples --example hello
cargo test  -p pedradb-examples --examples
```

`open → begin → get / put / delete / range → commit`. That loop is the whole
surface. Everything below is that loop, composed.

## The ladder

| # | Example | What it proves |
|---|---------|----------------|
| 1 | [`hello`](hello.rs) | Put, get, close, reopen. The write survived. |
| 2 | [`transactions`](transactions.rs) | Debit A, credit B, write audit — or none of them. Abort is real. |
| 3 | [`secondary_index`](secondary_index.rs) | Row + unique email in one commit. Half-index is impossible. |
| 4 | [`ordered_scan`](ordered_scan.rs) | Keys are bytes. Pad for numeric order. Prefix scan + pagination. |
| 5 | [`cas`](cas.rs) | `put_if_absent` / `compare_and_swap`. Stale holder cannot steal the lease. |
| 6 | [`snapshots`](snapshots.rs) | Overwrite a key; `get_at` still sees v1. Pin across the write. |
| 7 | [`change_feed`](change_feed.rs) | Puts and deletes as a durable `(from, to]` stream. |
| 8 | [`crash_reopen`](crash_reopen.rs) | Drop the handle after Ok; reopen; committed keys remain, aborted ones do not. |
| 9 | [`concurrent`](concurrent.rs) | Many threads, one `fdatasync` per group. OCC on disjoint keys; CAS on a hot counter. |
| 10 | [`bank`](bank.rs) | **Capstone.** Accounts, unique emails, transfers, ordered ledger, feed, reopen. |
| 11 | [`large_values`](large_values.rs) | Opt-in value log. Compact old SST versions, then `compact_vlog`. |
| 12 | [`backup`](backup.rs) | Base checkpoint + WAL ship + PITR restore. |
| 13 | [`rocksdb_dropin`](rocksdb_dropin.rs) | rust-rocksdb `DB::open_default` + `WriteBatch` + iterator, on Pedra. |

Read them in order once. After that, steal the file that matches the layer
you are writing.

## Complexity is in the keys, not the API

The engine stores bytes. A user row, an email index, a ledger entry, a lease,
a consumer cursor — they are all keys you designed. The discipline is:

1. **One transaction for one fact.** Row and index land together, or neither does.
2. **Length-prefix (or pad) components.** `idx/email/a` must not be a prefix of
   `idx/email/ab`. See `secondary_index` and `bank`.
3. **Lexicographic order is the query model.** `user/1`, `user/10`, `user/2`
   sort in that order. `ordered_scan` shows the fix.
4. **CAS is in the kernel.** Do not get-then-put a lease.
5. **Sequences are the clock.** Snapshots, the change feed, PITR, and OCC all
   pin the same counter `commit` returns.

`bank` is (1)–(5) in one main.

## What this is not

- Not a tutorial for Montanha (multi-Raft). The kernel is local on purpose.
- Not the internal probes under `crates/*/examples/` (those are benches and
  RFC harnesses).
- Not a claim that a 40-line ledger is a product. It is the *shape* of a
  product: you can build the rest without a second storage engine.

The engine contract is in the root [`README.md`](../README.md).
