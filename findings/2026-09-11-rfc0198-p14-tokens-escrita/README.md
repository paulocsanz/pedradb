# RFC-0198 P1.4 — tokens write_pending_frame pagos (board 0/17)

Fatia de coordenação resolvida SEM edição de `concurrent.rs`/`db.rs`
por este goal: o oráculo do board lê `unpaid_script=0/17` com os dois
tokens commitados no HEAD.

## O que o oráculo mostra (2026-09-11)

- `python3 .grok/skills/caminho-sel4/scripts/candidates.py`, seção
  script (rank 4): `finish_group_off_lock plan=wal_commit_plan
  calls_plan extra=ok` e `group_finish plan=wal_commit_plan calls_plan
  extra=ok`; `unpaid_script=0/17`, nenhuma linha UNPAID.
- Os tokens exigidos pelo board nos dois handlers:
  - `finish_group_off_lock` (`crates/pedradb-core/src/concurrent.rs`,
    tuple `("write_pending_frame", "sync_data", "fence_on_sync_fail")`)
    — call site commitado (a janela off-lock fd:
    `w.write_pending_frame().err().or_else(|| …)`).
  - `group_finish` (`crates/pedradb-core/src/db.rs`, tuple
    `("wal_sync_group", "write_pending_frame", "fence_on_sync_fail")`)
    — call site commitado (ramo `AppendApplyOk`:
    `self.wal.lock().write_pending_frame()?`).

## Por que o background dizia 2/17

O RFC foi escrito em 2026-09-10 contra o WORKTREE em voo da sessão
paralela (hunks não-commitados em `concurrent.rs` naquele momento).
Nenhum commit tocou `concurrent.rs`/`db.rs` entre o RFC (16b068c6) e o
HEAD de hoje; o conteúdo commitado paga os tokens desde a era
RFC-0041/RFC-0066 (`46a2556c` WAL write syscall off the Db write lock).
Verificado: `git log 16b068c6..HEAD -- concurrent.rs db.rs` = vazio, e
`git show HEAD:…` contém os dois call sites.

## Testes nomeados (mesmo commit)

`cargo test -p pedradb-core --lib -- concurrent::tests::failed_wal_sync_does_not_publish_group
concurrent::tests::multi_writer_failed_sync_does_not_publish_group`
→ 2 passed, 0 failed (drive o caminho `finish_group_off_lock`, que
contém o token).

## Disciplina de coordenação

Regra do goal: editar `concurrent.rs`/`db.rs` SÓ depois do commit da
sessão paralela. Mais forte que isso: NENHUMA edição foi necessária —
a condição de fechar era o board ler 0/17, e ele lê. O bloqueio
nomeado no RFC se resolve sem tocar o voo dela.
