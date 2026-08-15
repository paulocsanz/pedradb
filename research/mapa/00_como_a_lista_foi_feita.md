# How the first 100 was built

**Date:** 2026-08-14
**This is provenance, not a ficha.** None of the 100 except the PDFs already in `docs/references/` were read end-to-end for this library on this date.

## Inputs (primary enough to list, not to settle)

1. In-tree literature already persisted:
   - `docs/references/` (WiscKey, Monkey, Dostoevsky, Percolator, TiDB, HTAP set, Pebble notes)
   - `docs/engine-landscape-and-ideal-path.md`
   - `docs/rfc/0012-research-decisions.md`
   - `docs/htap-storage-primitives-and-research.md` §11
   - `docs/rfc/0010-dbs-on-top.md`, `docs/grail-plan-build-databases-on-pedradb.md`
   - `docs/rfc/0024-montanha-fold-for-caixote.md` + Slipstream sources
2. Lv, Li, Xu, Gao, Yang, Wang, Xue. *Rethinking LSM-tree based Key-Value Stores: A Survey*. arXiv:2507.09642, 13 Jul 2025. Bibliography of ~150 items, emphasis 2020–2025. Used as a **map**, not as a substitute for the papers it cites.
3. Venue programs / open PDFs: FAST, ATC, OSDI, SOSP, SIGMOD/PACMMOD, PVLDB, ASPLOS (2020–2026).
4. Confirmed 2025–2026 titles: Disco (SIGMOD'25), How to Grow an LSM-tree (SIGMOD'25), EcoTune (SIGMOD'25), CockroachDB Serverless (SIGMOD'25), veDB-HTAP (VLDB'25), HaSiS (FAST'25), PUSHtap (ASPLOS'25), TurtleKV (accepted VLDB'26), Leader Leases (SIGMOD'26).

## Ranking rule (applied by hand)

A paper entered the 100 if it scored on, in order:

1. Maps onto a Pedra crate, RFC, or an explicit non-ship (Lazy Leveling, skiplist, column-in-MANIFEST).
2. Peer-reviewed at FAST / OSDI / SOSP / SIGMOD / VLDB / ATC / NSDI / ASPLOS / TODS / CSUR, **or** is the canonical definition of a primitive (O'Neil LSM, ARIES, OCC, Raft, YCSB).
3. Recent (2020–2026) **or** so canonical that we keep re-deriving it.
4. Already cited in this repo (boost, not a free pass).

## What was excluded on purpose

- FPGA / GPU / DPU / CSD / Optane-only systems (backlog), except HaSiS/Polynesia/PUSHtap which are in the 100 as *negative* examples for Pedra hardware assumptions.
- Second and third papers from the same architecture when one representative exists (one OceanBase, one CRDB serverless, one Magma).
- Vendor blogs (Pebble, SlateDB, fjall, Antithesis). They live under `docs/references/` and `BACKLOG.md`.
- arXiv-only 2026 compaction papers without a confirmed venue (Tidehunter, Resystance, MountDB, FlintKV) — backlog until camera-ready.

## Bias to name

The list is **Pedra-centric**. A world-class learned-index paper can lose to a mediocre-looking compaction paper if the latter decides RFC-0014. That is intentional. Re-rank after fichas, not after citation counts.

## What was *not* done on 2026-08-14

- Downloading the 100 PDFs
- Reading the 100 PDFs
- Reproducing any number
- Treating Lv 2025 or Zhang 2024 survey sentences as theorems about Pedra

The next honest step is [`../queue.md`](../queue.md) Wave 0.
