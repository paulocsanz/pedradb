# RFC-0200 P0.1 — alcançabilidade da classe write-path (Inv-WAL completo)

Primeira fatia do RFC-0200. Fecha a frase seL4 completa do WAL que o
0198 deixou pela metade: "todo estado alcançável por QUALQUER
sequência de passos do write path satisfaz Inv-WAL" — não só
append-contas (`inv_wal_reachable`, 0198 P1.1), agora com
append/sync/ack intercalados em qualquer ordem (a física real do
group commit).

## O que mudou (`formal/aeneas/lean/WalState.lean`)

- `wal_write_step_reach` — família indutiva Nat-indexada: base `init`
  (`wal_state_init`, o log vazio) e passo `step` (qualquer construtor
  de `wal_write_step` — append n, sync h, ack n — estende a cadeia).
- `inv_wal_write_reachable` — COROLÁRIO: indução sobre a cadeia; base
  CITA `inv_wal_init` (0198 P1.1), passo CITA
  `wal_write_step_preserves_inv_wal` (0198 P1.2, o corolário da classe
  registrado). Nada re-provado — a prova inteira é duas citações.

## Verificação (mesmo commit)

- `lake build WalState` verde na primeira tentativa (1701 jobs);
  `grep sorry` = 0.
- `lean_extracts --required` não é executável neste instante sem RED
  do WIP não-commitado da sessão paralela (`ProbeLadderCount.lean`
  mid-proof + edit dela no script — documentado no sweep do 0198);
  o módulo tocado builda verde individualmente, mesma evidência do
  sweep. Sem mudança de registro (fatia de lema, não de escada).

## Contexto

Landado no mesmo intervalo em que a sessão paralela registrava (em
voo, não-commitado) o P0.3 do RFC-0199 — as duas linhas count para
`probe_order_covering`/`scale_predict` já estão no worktree dela; o
P2.1 do 0198 flipa quando o commit dela assentar.
