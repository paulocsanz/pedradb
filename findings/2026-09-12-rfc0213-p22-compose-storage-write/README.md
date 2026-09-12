# RFC-0213 P2.2 — composição ∀ do caminho de storage + sweep final

Data: 2026-09-12

## Composição (ComposeStorageWrite.lean — 22ª compose lib)

- `storage_write_path_recovered_iff`: o caminho de storage COMPOSTO
  — admission (`write_admit`) portão do append (sem Ok o write nunca
  chega ao WAL), plano (`wal_commit_plan`) cercando o sync requerido
  que falhou (Fence ⇒ sem publish), recovery (`torn_tail_needs_cut`)
  cortando a cauda torn — landa `ok v` com `v` true EXATAMENTE sob a
  conjunção quádrupla (sem stall mem, sem stall L0, sem cerca, fora
  da cauda torn), para TODO input. Cada perna deriva do seu atom
  registrado (`write_admit_fate_iff`, `wal_commit_plan_fate_iff`,
  `torn_tail_needs_cut_fate_iff`) — os corpos extraídos não são
  abertos; zero buracos.
- **Sem registro no TSV** — razão datada: uma linha exige par/entry
  único do catálogo; a composição atravessa três kernels/atoms
  (mesma regra das demais compose libs, ex. ComposeStoreFinish.lean
  0212 P2.2, ComposeStoreRaft.lean, ComposeL28.lean).
- Sem buracos: `grep -c sorry ComposeStorageWrite.lean` = 0;
  `lake build ComposeStorageWrite` verde ("Build completed
  successfully (1701 jobs)" com o kernel re-extraído); wiring:
  `[[lean_lib]] ComposeStorageWrite` no lakefile.toml + array
  COMPOSE no `scripts/lean_extracts.sh` — "ok lean extracts (64
  libs + 22 compose)". Twins no kernel ⇒ extrato re-carimbado via
  `aeneas_write_admission.sh --required` (SOURCE.write_admission +
  WriteAdmissionKernel.lean +49 linhas, os dois twins extraídos;
  wrapper inalterado, build verde no carimbo novo).

## Twin kernel DST (verde ANTES do commit)

- `write_admission_kernel::storage_write_recovered(…)`: a composição
  das três pernas sobre os kernels reais; o as-is composto
  `storage_write_recovered_as_is` porta as três mentiras de uma vez
  (admission sempre admite, plano nunca cerca, recovery nunca corta
  — write "sempre presente" após crash).
- `storage_write_recovered_on_live_stall_fence_torn_is_not_ok` —
  1 passed (pedradb-core, `cargo test --lib`, 0 failed): os quatro
  quadrantes do veredito (admitido+publicado+íntegro ⇒ presente;
  stall mem ⇒ recusado antes do WAL; sync requerido falhou ⇒
  cercado; cauda torn além do last-good ⇒ cortada) + o as-is
  composto mentindo nos três eixos simultaneamente.

## Sweep final (worktree destacado DENTRO de software/)

(preenchido no commit de sweep — capturas em `{SCRATCH}/r0213_p22_*.txt`)
