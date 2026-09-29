# F49 — Outstanding Queued proposes collide on durable `si_gen`

| Field | Value |
|-------|--------|
| **Status** | **FIXED** |
| **Site** | `StoreCluster::with_si_gen` / raft `Put`/`Batch` apply hist / `note_mutations` |
| **Class** | SI silent wrong (shared durable generation for distinct commits) |
| **Source** | `unit` (`queued_double_propose_distinct_si_gens_survive_reopen`) |
| **Repro** | `cargo test -p pedradb-store --lib queued_double_propose_distinct_si_gens_survive_reopen` |

## Symptom

`with_si_gen` **comment** said it reserved generations so concurrent proposes get
distinct numbers, but the body only computed `commit_generation + 1` **without
advancing** `commit_generation`.

Under `RpcMode::Queued` (multi-host / World path):

1. Client `put(A)` → `NotCommitted` — raft entry stamped `si_gen = G`.
2. Client `put(B)` before finish — also stamped `si_gen = G` (collision).
3. Raft apply writes `\0store/hist/` under the same gen for both keys.
4. Coordinator `note_mutations` later bumps memory gens independently (A@G,
   B@G+1), masking the bug on the happy path via `persist_si_keys`.
5. Crash after apply / any reload of **apply-path** hist collapses both commits
   into generation `G` → a snapshot that should see only the first commit also
   sees the second (**SI violation**).

Proved: both log entries embedded `si_gen=3` for indexes 4 and 5 after seed
puts (test assertion before fix).

## Fix

1. **`with_si_gen`**: advance `commit_generation` when stamping `Put`/`Batch`.
2. **`pending_version_notes`**: store `(si_gen, items)` with the reserved gen.
3. **`note_mutations_at(reserved, items)`**: apply reserved gen without a second
   bump (keeps memory OCC versions aligned with durable hist).
4. **`tx_finish`**: reserve one gen for the whole multi-range TX (F37) via the
   same advance-before-propose pattern; `note_tx_commit(handle, si_gen)`.

## Regression

`queued_double_propose_distinct_si_gens_survive_reopen` — asserts distinct
`si_gen` on leader log entries for two outstanding Queued puts, then SI at the
lower gen after finish/reopen.
