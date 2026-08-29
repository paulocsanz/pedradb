# RFC-0149 P1.1 — group-by-CF encode

`write_cf_owned` sorted by CF and reused the `cf\0` prefix buffer.
Apply prewrite was lock/default/lock/default ×32 (32 prefix allocs).

Isolated 3 rounds, same Rocks peer as 3x-baseline, deps only:

| shape | P0 median | P1.1 isolated med | vs Rocks |
|---|---:|---:|---:|
| deps_lock_prewrite | 1.849 | **2.092** | still <3 |
| deps_apply_batch | 1.914 | **2.075** | still <3 |
| deps_raftlog | 1.159 | 1.288 | still <3 |
| deps_scan | 2.840 | 2.678 | still <3 (noise) |

Phase stats (apply): mem **9.13µs**/commit of ~22µs wall — BTree `tail_idx` insert into a 267k-entry table. Cutting encode does not get these to 3×.

Blob (16 KiB vlog write) stayed ~1.16×. Official 12/17 >3× from P0 is unchanged.
