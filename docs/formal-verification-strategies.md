# Formal verification strategies — what fits Pedra, what to build

**Date:** 2026-08-15
**Question:** which verification strategies apply to Pedra / Montanha, and should we build our own?
**Answer:** do not build a verifier, a Coq/Dafny rewrite, or a TLA+ of “all of Raft.” Build (and keep tightening) the *method we already run*: extract the named `if`, make production call it, put teeth on the statement, prove the `fn`, simulate the axioms. That portfolio is what the papers that actually shipped systems recommend, once you subtract the language they happened to write in.

Canonical method already in the house:
[`determinismo/reports/formalizacao-e-prova-2026-08-14.md`](../../determinismo/reports/formalizacao-e-prova-2026-08-14.md)
and RFC-0002
[`determinismo/rfcs/0002-formalizacao-mista-ate-prova.md`](../../determinismo/rfcs/0002-formalizacao-mista-ate-prova.md).
Primary papers for this note live in
[`docs/references/formal-verification/`](references/formal-verification/SOURCES.md).

This note does **not** reopen “prove Pedra.” It classifies tools by *what object they talk about*, with numbers from the papers, then maps each object onto a Pedra kernel, a caller protocol, or an Env axiom.

---

## 1. What “formal verification” actually names

Every strategy below answers one of four different questions. Mixing them is how people claim “we verified the database” after checking a 200-line TLA+ sketch.

| Object | Question | Bound | Pedra analog |
|--------|----------|-------|--------------|
| **Design** | Do these state transitions implement this spec? | Finite instance (TLC) or ∀ in a proof assistant | A *new* protocol before it has a kernel (Parallel Commits, a fold pointer CAS) |
| **Production `fn`** | Does this Rust function match this `ensures`, for all inputs in its type? | ∀ of the type, relative to axioms | `vote_decision`, `ae_entry_action`, `txn_commit_action`, … |
| **Interleavings of that `fn`** | In every order ≤ B, does Inv hold? Does a mutant break it? | Bound B | Stateright on `vote_kernel` / `ae_kernel` |
| **Environment** | Did the disk / net / clock actually obey the axiom the proof assumed? | Sample of seeds | FailingEnv, World, det_io, tesoura |

IronFleet (SOSP’15, p. 1, §3) is explicit: TLA-style refinement for *protocol concurrency*, Floyd-Hoare for *one host’s imperative code*, reduction to pretend a host step is atomic. Hance et al. (OSDI’20, abstract + §3) then say the same sandwich applies to **storage**: the disk is just another asynchronous environment, a crash is a transition that resets the program and may tear unacked writes. Pedra already has that seam (`Env` / `FailingEnv` / crash-reopen). The missing piece in 2020 was “the program is Dafny.” In 2026 the program is Rust and the Hoare tool is Verus.

**Honest sentence we are allowed to use** (same as the 2026-08-14 report §8):

> Kernel K satisfies spec S; Verus/Lean accepted. Relative to axioms A. Change K so S fails and the machine refuses.

Never: “there are no bugs in Pedra.”

---

## 2. Strategy families (primary-source, not brochure)

### 2.1 Design model checking — TLA+ / PlusCal / TLC / Apalache / Quint

**What it is.** A few hundred lines of math that *is* the design. TLC enumerates a finite instance. Apalache does the same symbolically (SMT). Quint is TLA semantics with engineer syntax.

**What the papers actually measured.**

- AWS (Newcombe et al. 2014, pp. 3–7): ten production designs; specs **102–939 lines**. DynamoDB replication: learned TLA+ and wrote the spec **in a couple of weeks**; TLC on 10× `cc1.4xlarge` found a data-loss bug whose **shortest trace was 35 high-level steps**, after design review, code review, and fault-injection testing had all passed. Engineers learned TLA+ and got useful results in **2–3 weeks**. They brand it “exhaustively testable pseudo-code,” not “proof of the binary.”
- They are explicit about what TLA+ is **not** for (p. 5): emergent performance collapse, timeout retry storms, soft real-time. Those stay outside the logical spec.
- Cockroach Parallel Commits (2019 blog, cross-checked with our R045 ficha): a **week-long** internal TLA+ workshop produced `ParallelCommits.tla`. Properties: `AckImpliesCommit`, `ImplicitCommitLeadsToExplicitCommit`. This is *inventing* a commit condition, not re-proving Raft.
- MongoDB (2026-01-27 engineering post, companion to VLDB’25): **compositional** TLA+ — protocol vs WiredTiger as a formal interface. TLC of snapshot isolation on **2 tx × 2 keys** in ~10 minutes / 12 cores. Extra trick: *permissiveness* (how many legal SI histories the protocol allows), not just yes/no safety.
- Datadog Courier (2024): TLC **5,515,710** distinct states, `NoLostMsgs` green. Then they added a sequencer; the **model was stale**; TLC found a failure that in production they accepted as “sequencer runs every 3s.” They say keeping the model current for routine bugfixes is tedious; they only update it for large design changes.

**Fit.** Use when the `if` does not exist yet — a new 2PC, a new fold-export pointer rule, a new geo-commit. Do **not** TLA+ `vote_decision`: that function already is the design, in Rust.

**Twin risk.** High. The moment the Go/Rust handler diverges, TLC is proving last month’s protocol. Datadog and our own GlideFS `MODEL_AUDIT` are the same lesson.

### 2.2 Implementation model checking — Stateright, EXPLODE, MODIST, Kani

**Stateright** (library README + our `vote_model.rs` / `ae_model.rs`): the model *is* the Rust `fn`. Same crate runs in the checker and on UDP. Always / sometimes / (experimental) eventually. Linearizability tester inside the checker. Bound is still a bound.

This is the only model checker that matches the house rule “no paraphrase.” Slipstream’s `protocol.rs` + `tests/model.rs` is the same pattern.

**EXPLODE** (OSDI’06, abstract + §1): *implementation-level* model checking of **live** storage stacks. Principle: “explore all choices” at crash / `kmalloc` / bread. **36 bugs**, every system they checked (10 file systems, RAID, NFS, Berkeley DB, three VCS, VMware GSX), often without source. They rejected “rewrite the FS in a modeling language” as impractical. Pedra’s FailingEnv + crash-reopen is the in-process cousin; det_io / QEMU is the live-stack cousin. EXPLODE is **bug finding**, not ∀.

**Kani** (Amazon, CBMC backend): bit-precise bounded model check of the **compiled** Rust, including overflow and a subset of `unsafe`. Already used in `rbs-dst/kani-proofs` for LBA/offsets. Complements Verus: Kani catches wrap that a `u64` spec treated as math `int`. RFC-0002 P2.3 already did a Kani-style ∀u16 grid on vote persist. Use Kani on `len_pref` / pack / WAL codecs, not on Raft interleavings.

**Loom / Shuttle:** thread schedules. Right tool for `ConcurrentDb` locks. Wrong tool for vote.

### 2.3 Deductive verification of the production language — Verus, Creusot, Prusti, Dafny, F\*

**IronFleet** (SOSP’15, §7): first machine-checked **safety + liveness** of non-trivial distributed *implementations*. IronRSL trusted spec **85 SLOC**, IronKV **34**. Implementation proof:code **3.6 : 1**. IronRSL implementation **5,114** lines, proof **39,253**. Full serial verify **~6 hours**; incremental **6–8 minutes**. Methodology + two systems: **~3.7 person-years**. Throughput within **2.4×** of unverified MultiPaxos. TCB: spec, main loop, Dafny, .NET, Windows (they point at Ironclad for assembly). Liveness assumes a live quorum and eventual synchrony. They verify **newly written Dafny**, not existing C++.

**Hance / VeriBetrKV** (OSDI’20): apply IronFleet to a crashy disk. Application spec **283 lines**: dictionary that, on crash, reverts no farther than last `sync`. Disk may corrupt any time except it cannot mint a valid CRC32C (checksum is in the TCB). Insertions **24×** BerkeleyDB, **8× slower** than RocksDB. **98.3%** of definitions verify in **<10 s** under a timeout-squash discipline. Single-threaded. They **refused** Crash-Hoare / a custom crash logic: crash is just another IOSystem transition.

**Verus SOSP’24** (abstract, §1, §4–5): same IronFleet/VeriBetrKV lineage, but the host language is **Rust**. Case studies: **6.1k impl + 31k proof**; verification **3–61×** faster than prior SMT tools. New systems: page table proof:code **13.3 : 1**; mimalloc-subset **4.3 : 1** (~17.2k lines, whole project verifies in **~1 minute**); persistent log **3.9 : 1**, verifies in **12 s**, **integrated into Azure Storage as a Cargo crate**. TCB: specs, Verus, solvers, rustc. Explicitly does **not** bake crash-safety into the tool — you model it as a state machine (same as Hance). EPR mode (Ivy-style) can be opted into per module and composed with unrestricted proofs (IronKV delegation map: ~300 lines of default-mode proof replaced by an auto-checked EPR invariant).

**Creusot / Prusti:** Why3 / Viper. Verus SOSP’24 §5: slower SMT, no ghost-resource story comparable to VerusSync, no EPR, no concurrency story at Anvil/NR scale. Not a reason to switch.

**Fit.** Verus on **kernels we already extracted** is the cheap end of this family ( Pedra’s vote twin is 1 `ensures`, not 39k lines of IronRSL). Verus on `db.rs` / `ConcurrentDb` / the Raft event loop is IronFleet-scale (person-years, 3–13× proof). Do not start that without a named protocol layer and a reduction argument.

### 2.4 Extract-and-prove — Verdi, Aeneas, hax, CompCert-style

**Verdi** (PLDI’15): write the system **in Coq**, prove it against an explicit network semantics, extract OCaml. Raft linearizability: spec **170**, impl **520**, proof **4,144** (Table 2). Lock-service mutex proof **~500 lines**. TCB: spec, network semantics ≟ physical net, OCaml shim, Coq, ocamlopt. They prove **their** Raft, not etcd’s. Performance of `vard` vs etcd in their microbench is close; they say etcd has better structures and batching. This is the opposite of our constraint: **production Rust is the source**.

**Aeneas** (ICFP’22, pp. 1–3): Charon lowers safe Rust to LLBC; Aeneas emits a **pure** λ-term for Lean / Rocq / F\* / HOL4. **Safe, sequential only.** No `unsafe`, no interior mutability. Handlers that `persist_hard` do not extract. Our kernels do. There are **no** Lean hand twins in this repo (that claim was stale). The extract target is `formal/aeneas/vote-kernel/` (`include!` of production `vote_kernel.rs`). Charon/Aeneas themselves are **not pinned on this host** until `formal/aeneas/PINS.md` has a rev and `./scripts/aeneas_vote.sh` has produced `formal/aeneas/out/`.

**Fit.** Aeneas is the *second machine* for a kernel, when Charon is pinned. It is not a path for `handle_request_vote`. Until extract is mechanical, a Lean file is a third twin (production / Verus / Lean) and needs a drift check, not a “two machines” claim.

### 2.5 Simulation as the axiom lab — FDB Sim, Pedra World, TigerBeetle VOPR

FoundationDB (SIGMOD’21 + public testing doc; our R043 ficha): 18 months of simulation **before** real disk. The binary *is* the model because Flow is compiled both ways. That is DST, not a theorem. AWS 2014 used TLA+ **and** a simulated network for DynamoDB; they still needed TLA+ for the 35-step design bug.

Pedra already chose this floor for axioms (RFC-0018). Datadog (2024) wanted DST for Courier and bounced off Go runtime nondeterminism — the reason Pedra’s `Clock` / `Rng` / `Env` / Queued RPC exist.

Simulation never becomes a ∀. After a kernel is proved, a SilentWrong seed is either (a) an axiom lie or (b) a caller that does not refine the kernel or (c) twin drift. That is the only way the two floors talk.

### 2.6 Domain-specific logics we should not adopt

| Logic | Why it exists | Why not here |
|-------|---------------|--------------|
| Crash-Hoare / FSCQ / Perennial | Crash mid-function in Coq | Hance OSDI’20 §2.3–3: model crash as an IOSystem step; no new logic. Pedra already has that step. |
| Iris / Grove | Concurrent / distributed sep. logic in Coq | Heroic; Verus SOSP’24 built VerusSync so systems people do not write monoids. Grove not read in full this session. |
| P / P# (AWS, 2019–) | State-machine language + testers + (2025) PObserve | Another design language. Same twin problem as TLA+ unless we generate Rust from P, which we will not. |
| Alloy | Relational bounded check | AWS 2014 pp. 6–7: not expressive enough for nested records / sequences they needed. Chord is the success story, not a storage engine. |

---

## 3. What databases actually do (not what keynotes imply)

| System | Design check | Impl check | Theorem of the binary | Axiom lab |
|--------|--------------|------------|------------------------|-----------|
| **FoundationDB** | almost none publicly | — | — | Sim (the gold standard) |
| **AWS S3 / DynamoDB / EBS** | TLA+ 100–900 LOC; later P | — | Cedar is a different story | fault injection + (2014) simulated net |
| **Cockroach** | TLA+ of Parallel Commits (1 week) | — | — | kvnemesis / DST-ish |
| **MongoDB txns** | compositional TLA+ vs WiredTiger | model-based tests of WT (part 2, unread PDF) | — | — |
| **etcd / TiKV Raft** | Ongaro’s `raft.tla` is the *paper* Raft | — | — | Jepsen, failpoints |
| **IronRSL / IronKV** | TLA-style in Dafny | Dafny **is** the impl | yes, of Dafny | assumed net |
| **VeriBetrKV** | TLA-style crash spec | Dafny impl | yes, of Dafny | disk model |
| **Verus persistent log** | crash SM | Verus Rust | yes, of that crate | CRC axiom |
| **Anvil controllers** | TLA embedding in Verus | Verus Rust | liveness ESR | env model |
| **Pedra (now)** | pages in `pedradb-dst/formal/` | kernels + Stateright on the catalog models | Verus twins (`close`/`atom`/`model`); Aeneas include crate, no Lean extract yet | World / FailingEnv / DST |

Nobody running a production LSM in C++/Go/Rust has a Verdi-style “the extracted binary *is* the proof term” for the engine. The ones who have theorems either (a) wrote the system in the prover’s language or (b) verified a **small Rust crate** (Azure log, Anvil controller, our kernels).

---

## 4. Fit to Pedra, layer by layer

```
                    ∀ kernel            bound              sample
high-level spec  ─ Verus/Lean ─  Stateright/TLC ─  DST/World
     ▲                  │                │                │
protocol step  vote/ae/commit/txn kernels (production calls these)
     ▲                  │
caller refine  grant_after_persist, propose_ack_ok   ← still a kernel
     ▲
I/O + time     persist_hard, fsync, CRC, net, clock  ← axioms forever
```

| Pedra object | Right tool | Already? | Do not |
|--------------|------------|----------|--------|
| `vote_decision` / `log_up_to_date` | Verus ∀u64 + Aeneas extract when Charon is pinned | Verus close twin; include crate ready; no `.lean` yet | TLA+ rewrite |
| Multi-node vote / AE orders | Stateright on the **same** `fn` | yes (2 models) | TLC of a PlusCal Raft |
| `txn_commit_action`, abort fence | Verus + one more Stateright | Verus yes; no Stateright | IronFleet 2PC |
| Majority ACK (`propose_ack_ok`) | Verus on the predicate | yes | “prove I-MAJ of the cluster” in Lean |
| WAL recover / torn tail | DST + EXPLODE-style choice points | FailingEnv; not systematic `choose` at every I/O | put CRC in the theorem |
| Journal pin-on-read (H1) | **new kernel** + mutant + Verus | no | a TLA+ of NATS |
| HTTP parsers | Verus on tiny predicates | yes (F79–F105) | more of these first |
| `ConcurrentDb` | Loom / TSan | residual | VerusSync of the whole engine |
| New fold pointer / 2PC variant | Quint/TLA+ **then** extract kernel | not yet needed | skip the kernel and keep only TLA+ |
| Full Montanha Raft + store | IronFleet layers (person-years) | not started | “we’ll Verus `lib.rs`” |

Hance’s StorageIOSystem is literally Pedra’s picture: program steps, Env steps, crash resets the program and may tear unsynced WAL. The application spec they used (dictionary + sync fence) is the right *shape* for a future `Db` theorem. It is **not** a weekend slice. VeriBetrKV was a dedicated verified KV with a 283-line spec and a multi-level refinement; Pedra’s LSM is larger and already shipping.

---

## 5. Should we build our own?

### Never build

| Artifact | Why |
|----------|-----|
| SMT solver / Verus competitor | Z3 + Verus are multi-institution, decade-scale (Verus SOSP’24). |
| Lean/Rocq kernel | Same. |
| TLC/Apalache replacement | Exists; Quint exists if we hate TLA+ syntax. |
| A Pedra language that extracts to Rust **and** Lean | Verdi/EventML/P. We already refused “rewrite the daemon.” |
| Crash-Hoare / Iris embedding | Hance + Verus persistent log show crash as a state machine is enough. |

IronFleet took **3.7 person-years** for two Dafny systems. seL4’s early proof:code was **>20 : 1** (Verus SOSP’24 §5). That is the cost of “prove the implementation,” not of “prove the if.”

### Already built (this *is* our own, and it is the valuable part)

The house ritual is a verification *strategy*, not a tool:

1. Finding → named invariant + axioms + drift-traps (page).
2. Pure kernel; production calls it.
3. Table tests + AS-IS mutant that reproduces the REAL.
4. Stateright / finite domain when the `if` is about orders.
5. World / FailingEnv whose oracle **is** the page id (F15, …).
6. Verus `ensures` on a twin; optional Lean.
7. DST keeps attacking the axioms.

Beyond has (1–4). IronFleet has (6) on a different language. AWS has a weaker (1) + TLC. FDB has (7) only. **Nobody we read ships 1–7 on the production Rust `fn`.** That combination is ours. Do not throw it away for a more famous logo.

### Worth building (thin tooling, not a prover)

These close residuals the papers all warn about (twin, timeout, stale model):

1. **Twin-faithfulness CI** — hash/diff production kernel vs Verus twin (and Lean, if kept). IronFleet’s whole pitch is “no semantic gap between protocol and impl because both are Dafny.” We split languages; the gap is back. The vote script comments about a match-arm diff; the script as checked in only runs `verus`.
2. **`pedra-formal` harness** — one entry that runs `verus_*.sh` + Stateright tests + mutant discovery assertions. Optional on GitHub (Verus is not on the runner today).
3. **Kernel lint** — production path must call the kernel (grep/handler audit). A kernel that only tests call is a twin by another name (Verdi related-work criticism of “formality gap”; GlideFS MODEL_AUDIT).
4. **Choice-point crate for recover** — EXPLODE `choose(N)` at WAL fragment / CRC / short-read, driven by DST. This is *our* EXPLODE, 50 lines, not a research OS.
5. **Quint/TLA+ only as a *design* scratchpad** for the next protocol that is not yet a kernel (fold artifact pointer, a new 2PC). Delete or freeze the spec the day the kernel lands.

That is “build our own” in the sense FDB built Flow+Sim and AWS built a TLA+ culture — **a method and a few hundred lines of glue**, not a new logic.

---

## 6. Recommended portfolio (priority, not a rewrite)

Keep the three floors. Spend new effort only where a floor is empty.

Harness (shipped): `./scripts/pedra_formal.sh` — catalog in `scripts/formal/catalog.json`. CI job `formal-glue` runs `--ci` (lint, clones, twins, Stateright). Verus stays optional unless `--verus-required`.

| Priority | Slice | Floor | Why this and not something else |
|----------|-------|-------|----------------------------------|
| P0 | Twin-diff CI + run `verus_*` on a machine that has Verus | 3 | **Glue landed.** Five missing twins filled (index_val / isolated / children / fields / pack + `pack_kernel`). `--verus-required` not on GitHub yet. |
| P0 | Journal pin kernel (apply then persist pin) | 1→3 | **Landed:** `pin_kernel` + Verus twin + `pin_model` (peek must not pin ahead of apply). Canary `catch_up` still pins on read by contract. |
| P1 | Stateright on `txn_kernel` or `discard_cut` | 2 | **Landed:** `crates/pedradb-store/tests/txn_model.rs` (F47 fence, discard cut, F34 preimage). |
| P1 | EXPLODE-style `choose` in WAL recover, DST-driven | 1 | **Landed:** kernel + `recover_model`; **byte `choose`:** `wal/recover_choose.rs` + `recover_choose` sweep + `FaultEnv::choose_wal_recover` (CRC/orphan fail-stop, length resync). |
| P1 | Stateright on `lease_kernel` / `compact_kernel` | 2 | **Landed:** `lease_model` (F7 unknown/reuse, F56 clock reset) + `compact_model` (F28 offline floor, F27 missing term). |
| P1 | Stateright on `snapshot_kernel` / `dcs_apply` | 2 | **Landed:** `snapshot_model` (F38/F41 reserved keys, F40 txn-meta clear) + `apply_model` (F12/F22 CasFailed advances). |
| P1 | Stateright on `commit_kernel` | 2 | **Landed:** `commit_model` (F10 recover/reapply, F23 current-term, F11 ACK). Store body is the catalog clone. |
| P1 | Stateright on `si_kernel` | 2 | **Landed:** `si_model` (F42 partitioned `ids[0]`, F84 range `applied` vs global seq). |
| P1 | Stateright on stream `cursor_kernel` | 2 | **Landed:** `cursor_model` (F54 ack-in-order, peek must not pin). |
| P1 | Stateright on HTTP `fail_closed` | 2 | **Landed:** `fail_closed_model` (F102 mute, F104 TE, F105 bad int, F153 LF break, F154 Expect 100). |
| P1 | Stateright on HTTP `cl_kernel` | 2 | **Landed:** `cl_model` (F86 keep-without-CL, F87 invalid≠0, F88 repeat, F146 short body). |
| P1 | Stateright on HTTP `form_kernel` | 2 | **Landed:** `form_model` (F101 `+`→space, F155 query conflict). |
| P1 | Stateright on HTTP `path_kernel` | 2 | **Landed:** `path_model` (F91/F92 authority strip, F156 fragment). |
| P1 | Stateright on fold `isolated_kernel` | 2 | **Landed:** `isolated_model` (F83 `/vm/vm-a` vs sibling `/vm/vm-ab`). |
| P1 | Stateright on `prefix_exclusive_end` | 2 | **Landed:** `prefix_model` (F57/F58 `prefix||0xff||…` stays in `[prefix, end)`). |
| P1 | Stateright on `range_tombstone_covers` | 2 | **Landed:** `range_model` (F30 `[start, end)` covers interior; AS-IS only start). |
| P1 | Stateright on `changelog_needs_sst_rebuild` | 2 | **Landed:** `changelog_model` (F53 empty feed + live seq rebuilds after flush). |
| P1 | Stateright on `pack_cut_tag` | 2 | **Landed:** `pack_model` (F62 `pack([a\\0b,c])` ≠ `pack([a,b\\0c])`). |
| P1 | Stateright on `children_kernel` | 2 | **Landed:** `children_model` (F59 zip `90` vs `900`; end is `||0x01` not `||0xff`). |
| P1 | Stateright on `fields_kernel` | 2 | **Landed:** `fields_model` (F60 NUL inside zip/id; length-prefix keeps, first/last-NUL truncates). |
| P1 | Kernel + Stateright on replicate rotation guard | 2 | **Landed 2026-08-15:** `ship_kernel::pull_plan` (F165 flush rotates `CURRENT.log` in place; length-only check shipped misaligned bytes / silent stale replica; vanished WAL read as up-to-date). Prefix stamp (`ship_model` + Verus `ship_guard`, model domain) fails closed on shrink, rewrite, and vanish. |
| P1 | Fail-closed bloom header bound | 2 | **Landed 2026-08-15:** `bloom_header_ok` (F166 on-disk `k` untrusted — corrupt near-`u32::MAX` probes made every `SstTable::get` loop billions of times). Decode rejects `k > MAX_K`; `bloom_model` + Verus `bloom_header` (close twin) prove accept ⇒ bounded probes / bits cover `nbits` / bits fit buffer. |
| P1 | SST scan fast-reject honors spanning tombstones | 2 | **Landed 2026-08-15:** `scan_kernel::scan_reads_file` (F167 `entries_in_user_range` whole-file prune used point bounds only — a range tombstone's end key lives in the value, so a file whose points all precede the window was skipped and covered keys scanned live while `get` said `None`). Kernel keeps the file when a tombstone straddles the window (end past start, start before end — sharp for after-window files); `scan_model` + Verus `scan_guard` (model domain) prove skip ⇒ no window key covered. |
| P1 | SI snapshot reads fail closed below the GC floor | 2 | **Landed 2026-08-15:** `si_kernel::snapshot_read_plan` (F168 `get_at_version` served snapshots older than the GC floor from pruned history — committed data read back as `Ok(None)`, fabricated absence, while `TransactionTooOld` was only raised at commit). Kernel rejects `snapshot < watermark-1` (overflow-free); `si_read_model` + Verus `si_read` (close twin) prove TooOld ⇒ floor cannot cover, Serve ⇒ servable, no overflow reject at `u64::MAX`. |
| P1 | Fold last-per-key / apply honor range tombstones | 2 | **Landed 2026-08-15:** `fold_kernel::fold_event_hides_key` (F169 changelog `DeleteRange` was mapped to a point delete of the start key; `last_per_key` last-write-wins left covered puts live while source `get` hid them). Kernel uses half-open `[start, end)`; dest `FoldUpdate::DeleteRange` expands against live dest keys + same-batch ops. `fold_range_model` + Verus `fold_range` (close twin) prove range ⇒ cover, outside ⇒ keep, point ⇒ exact. |
| P2 | Aeneas extract of `isolated_id_matches` (F83 cartoon) | 3 | **Landed 2026-08-15:** axiom-free extract + Lean ∀ `isolated_id_matches_spec` and `as_is_leaks_sibling`. |
| P2 | Aeneas extract of `vote_decision` when Charon is pinned | 3 | **Landed 2026-08-15:** extract + Lean accepted `vote_decision_matches_spec`, `grant_after_persist_implies_ok`, AS-IS teeth. Persist/disk still axioms. |
| P1 | Bloom filter T1–T4 (beyond F166 header) | 2–3 | **Landed 2026-08-15 (RFC-0030):** production `insert`/`may_contain`. Exhaustive domain + teeth. Verus model-domain twin (7 verified). Kani on compiled production: T1 (every 2-byte key, 64-bit/k=2 filter, 2959 checks) + T4 (337 checks). Aeneas extract of production `bloom.rs`; Lean accepted T4 and T1 core `set_bit_test_bit_same`. Loop-level insert/query on the extract and Kani T2/T3 not claimed. |
| later | IronFleet-style refinement of `StorageIOSystem ⊨ sync-fence map` | 3 | Hance’s theorem. Person-year class. Only after kernels cover commit+flush+MANIFEST publish. |
| never | Verdi rewrite; P language; prove Linux fsync; “TLA+ of Montanha” as a substitute for kernels | — | Papers we read either did not do this or spent years and still assumed the disk. |

---

## 7. Claims this note does **not** make

- That Verus is sound (they do not verify Verus or rustc; guide + SOSP’24 TCB).
- That our Verus twins are the production functions (they are not, until twin-CI exists).
- That IronFleet liveness is available to us (needs fairness axioms + a reduction of the event loop).
- That MongoDB VLDB’25 numbers were page-checked (PDF 403).
- That AWS 2025 P/PObserve was page-checked (ACM wall).

---

## 8. One sentence

The strategies that shipped systems all do the same split IronFleet named in 2015 and Hance applied to disks in 2020: **prove the host step, model-check the protocol, simulate the environment.** Pedra already split the host step out as kernels and proved many of them. Building “our own verification” means owning that split — glue, mutants, twin-CI, choice points — not writing a prover.
