# RFC-0053 Y1 report

**Date:** 2026-08-23  
**Status:** Y1 slices P0–P2.3 landed. **Not** a ∀ crash-dictionary on `Db`. **Not** “Pedra verificado.”

## LOC (this tree)

| Object | Lines |
|--------|------:|
| Production kernels (vote + AE + commit) | 784 |
| Verus twins (raft vote/AE/commit + WAL recover) | 886 |
| Lean theorem files (Vote + Ae + Commit) | 214 |
| `db.rs` | 14 420 |
| `ConcurrentDb` | 4 846 |
| `pedradb-store` `lib.rs` | 10 805 |
| `pedradb-raft` `lib.rs` | 1 770 |

Proof:kernel on the three raft kernels ≈ 886/784 ≈ **1.1 : 1** (Verus twins) plus Lean on the Aeneas extract. IronRSL was 3.6 : 1 on a Dafny rewrite — we are not claiming that rácio on `db.rs`.

## TCB (Y1)

**In:** pure kernels production calls (`vote_decision`, `grant_after_persist`, `ae_entry_action`, `recover_commit`, `may_commit_at`, `wal recover_collect_act`, …); Verus twins; Aeneas extracts (`VoteKernel`, `AeKernel`, `CommitKernel`) + Lean theorems; rustc; Verus+Z3; CRC “not forgeable”.

**Axioms forever:** `persist` Ok\|Err; `fsync` / OS; net delivery; clock.

**Out until Y3 / RFC-0051:** `ConcurrentDb` (group-commit, OS threads); `montanha-tcp` accept/health/event loop; io_uring live ring; libc.

## What DST still covers

Crash after Ok, torn WAL tail, CRC fail-stop, silent_wrong seed matrix (`docs/formal/crash-dictionary.md` teeth). World cluster schedules (RFC-0050, still sibling). π on `ConcurrentDb` (RFC-0051). Linux det_io / TCG (RFC-0052).

A SilentWrong seed after a proved kernel is an **axiom lie**, a **caller that does not refine**, or **twin drift** — not a hole in `vote_decision_iff`.

## Claims

| Allowed | Forbidden |
|---------|-----------|
| Extract of vote/AE/commit has named Lean theorems, no `sorry` | dicionário ∀ / `Db::put` proved |
| `grant_after_persist` ⇒ persist Ok (Verus) | ConcurrentDb in TCB |
| Stateright compose_model FIXED + AS-IS teeth | “Pedra verificado” |
