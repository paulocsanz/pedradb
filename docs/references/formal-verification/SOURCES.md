# Formal verification primary sources (this directory)

Fetched 2026-08-15 for
[`docs/formal-verification-strategies.md`](../../formal-verification-strategies.md).
PDFs are the copies used for page citations. Abstracts / blogs are
secondary unless noted.

| File | Paper / artifact | Venue | Read this session |
|------|------------------|-------|-------------------|
| `ironfleet-sosp15.pdf` | Hawblitzel et al., *IronFleet: Proving Practical Distributed Systems Correct* | SOSP 2015 | pp. 1–6, 12–16 (method + eval + related) |
| `verdi-pldi15.pdf` | Wilcox et al., *Verdi* | PLDI 2015 | pp. 1–6, 10–12 (method + Table 2 + related) |
| `hance-osdi20-storage.pdf` | Hance et al., *Storage Systems are Distributed Systems (So Verify Them That Way!)* | OSDI 2020 | pp. 1–8 (method + VeriBetrKV + limits) |
| `verus-sys-sosp24.pdf` | Lattuada et al., *Verus: A Practical Foundation for Systems Verification* | SOSP 2024 | pp. 1–8, 12–16 (design + eval + related) |
| `verus-ghost-oopsla23.pdf` | Lattuada et al., *Verus: Verifying Rust Programs using Linear Ghost Types* | OOPSLA 2023 | downloaded; not re-read in full (SOSP’24 supersedes for systems claims) |
| `aws-tla-2014.pdf` | Newcombe et al., *Use of Formal Methods at Amazon Web Services* | 2014 preprint of CACM 2015 | pp. 1–8 |
| `explode-osdi06.pdf` | Yang, Sar, Engler, *EXPLODE* | OSDI 2006 | pp. 1–3 |
| `anvil-osdi24.pdf` | Sun et al., *Anvil: Verifying Liveness of Cluster Management Controllers* | OSDI 2024 | pp. 1–3 |
| `aeneas-icfp22.pdf` | Ho & Protzenko, *Aeneas: Rust Verification by Functional Translation* | ICFP 2022 / arXiv 2206.07185 | pp. 1–3 |

## Read as HTML (not archived as PDF)

- Cockroach Labs, *Parallel Commits* (2019) — TLA+ of a **new** commit protocol; spec later in-tree as `docs/tla-plus/ParallelCommits/`.
- MongoDB, *Formal Methods Beyond Correctness* (2026-01-27) — compositional TLA+ at the WiredTiger boundary; companion to VLDB’25 (PDF 403 this session).
- Datadog, *How we use formal modeling, lightweight simulations, and chaos testing* (2024) — TLA+ of Courier; TLC 5,515,710 states; model went stale after a sequencer was added.
- Verus guide `overview.html` (2026) — TCB: does not verify verifier or rustc/LLVM.
- Stateright README (github.com/stateright/stateright) — same Rust runs in checker and on the network.

## Not fetched / blocked this session

- AWS Brooker & Desai, *Systems Correctness Practices at Amazon Web Services* (CACM/Queue 2025) — ACM paywall/bot wall. Claims about P / PObserve taken from Queue HTML snippets + P project README, **not** treated as page-checked.
- MongoDB VLDB’25 PDF (`p5045-schultz.pdf`) — HTTP 403.
- Grove (SOSP 2023), Perennial/FSCQ, Ivy CAV papers — cited via Verus/Hance related-work only.

## In-tree Pedra / determinismo (already canonical)

- `determinismo/reports/formalizacao-e-prova-2026-08-14.md`
- `determinismo/rfcs/0002-formalizacao-mista-ate-prova.md`
- `determinismo/pedradb-dst/formal/P1.4-vote-theorem.md`
- Pedra kernels + Verus twins under `crates/*/verus/` and `scripts/verus_*.sh`
- Aeneas include crate: `formal/aeneas/` (no extracted `.lean` in-tree as of 2026-08-15)
