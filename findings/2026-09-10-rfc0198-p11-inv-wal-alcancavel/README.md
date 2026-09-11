# RFC-0198 P1.1 — Inv-WAL base inicial + corolário de alcançabilidade

Primeira fatia indutiva (forma seL4) do RFC-0198. Peer de referência:
nenhum (fatia formal, sem bench).

## O que mudou (`formal/aeneas/lean/WalState.lean`)

- `wal_state_init` — estado inicial do WAL (log vazio: os três
  watermarks em zero; o que `wal_state_of 0 0 0` constrói na produção).
- `inv_wal_init` — BASE: o log vazio satisfaz Inv-WAL
  (zero ⊆ zero ⊆ zero; `unfold + rfl` no mesmo estilo do teorema de
  catálogo já existente).
- `wal_append_reach` — predicado indutivo Nat-indexado: um estado é
  alcançável quando há cadeia de n `wal_append` ok a partir do inicial.
- `inv_wal_reachable` — COROLÁRIO: todo estado alcançável por n appends
  satisfaz Inv-WAL. Base = `inv_wal_init`; passo = lema um-passo
  REGISTRADO `wal_append_preserves_inv_wal` (RFC-0191 P2.1), CITADO —
  o indutivo só encadeia, não re-prova o passo.

## Verificação (mesmo commit)

- `lake build WalState` verde (1701 jobs); `grep sorry` no arquivo = 0.
- Gates depth/product/ledger GREEN no commit (sem mudança de registro —
  fatia de lema, não de escada).

## Nota de coordenação

P0.2 ficou BLOQUEADO no mesmo intervalo: a sessão paralela opera
`proof_depth.tsv`/`residuals.json`/`check_depth_floor.py` não-commitados
(dimensão `floor_count` do RFC-0199) e um `git reset` estranho limpou o
insert do teorema de GroupCommit.lean. O teorema P0.2 está PROVADO e
preservado em `theorem.snippet.lean` neste diretório; landa depois do
commit dela. Este commit de P1.1 não toca nenhum arquivo compartilhado.
