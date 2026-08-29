# F-L28 — RequestVote skipped for remote members

**Status:** FIXED (2026-08-26)  
**Seed / test:** `l28_real_tcp_seed_replay` (`0x641e28`)  
**Cluster:** 3× `montanha-tcp` OS processes, `cluster_real`

## Symptom

REAL TCP cluster never elected (`r1:leader=-` for 45s). In-process World still elected.

## Cause

RFC-0064 `vote_targets` loop skipped `pid` when `!nodes.contains_key(pid)`.
On the multi-host path each process holds **one** local node. Remotes are
in `ids` only. No RequestVote left the process.

## Fix

Send RV to every vote-target except self; skip only a **local** node with
`participating == false`.

## Regression

`cargo test -p pedradb-store --test l28_real_tcp` — same seed twice:
`put=1 get=1 kill=1 restart=1`.
