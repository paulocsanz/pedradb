# RFC-0198 P1.2 — Inv-WAL preservação pelo passo sync/fence (e ack)

Segunda fatia indutiva do RFC-0198: fecha a frase "todo passo do write
path que toca o WAL preserva Inv-WAL". O P1.1 já tinha append (via
alcançabilidade); aqui entram os outros dois operadores do kernel
`WalState` — `wal_sync` (a barreira/fence, nas duas honestidades do
Env) e `wal_ack` (o Ok pro cliente) — e o corolário que fecha a classe
completa. Peer de referência: nenhum (fatia formal, sem bench).

## O que mudou (`formal/aeneas/lean/WalState.lean`)

- `wal_sync_honest_closed` (privado) — forma fechada: sync Honest =
  `ok {s with synced := s.written, written := s.written}`. O Env
  honesto promove a barreira exatamente até o watermark escrito, nunca
  além.
- `wal_sync_lying_closed` (privado) — forma fechada: sync Lying =
  `ok {s with synced := min s.synced s.written (por .val), written :=
  s.written}`. O Env mentiroso devolve o próprio estado recortado pelo
  `CrashModel.of` (o `min` — a física do crash nunca amplia synced).
- `wal_sync_preserves_inv_wal` — qualquer desfecho `ok` de `wal_sync`
  preserva Inv-WAL, nas duas honestidades. Estratégia: os dois lemas
  fechados reduzem cada ramo a `{s with synced := w, written :=
  s.written}`; um lema auxiliar `finish` mostra que QUALQUER w com
  `synced ≤ w ≤ written` preserva o invariante (o conjunto de estados
  seguros é upward-closed entre as duas barreiras); os ramos aplicam
  `finish` com `w = written`, `w = synced` e `w = written`,
  respectivamente.
- `wal_ack_preserves_inv_wal` — qualquer desfecho `ok` de `wal_ack`
  preserva Inv-WAL. O ack só avança `acked` quando o add saturado cabe
  em `synced` (ramo `ok`); quando o add checado estoura, o passo nem é
  `ok` — logo o Ok pro cliente nunca quebra `acked ⊆ synced`. Os ramos
  fail/div contradizem a hipótese `ok`.
- `wal_write_step` — família indutiva indexada por (estado, estado):
  um passo do write path é append n | sync h | ack n, cada construtor
  carregando o `ok` do operador.
- `wal_write_step_preserves_inv_wal` — COROLÁRIO da classe completa:
  cada construtor CITA o lema um-passo correspondente
  (`wal_append_preserves_inv_wal` RFC-0191 P2.1; os dois novos deste
  commit); nada é re-provado aqui.

## Verificação (mesmo commit)

- `lake build WalState` verde; `grep sorry` no arquivo = 0.
- `bash scripts/lean_extracts.sh --required` verde (61 libs + 5
  compose). Os warnings de `sorry` do output são herdados da lib
  Aeneas (`Aeneas/Std/Slice.lean`, `StringIter.lean`), não deste
  arquivo.
- Gates depth/product/ledger sem mudança de registro (fatia de lema,
  não de escada) — arquivos de registro não tocados.

## Armadilhas Lean resolvidas (para a próxima fatia indutiva)

- Instância de record multi-linha é sensível a whitespace: campos
  depois do `with` precisam de linha própria indentados mais fundo que
  o `{`.
- `cases h : p` numa prop que alimenta um `if` falha ("generalize
  failed: result is not type correct") — o caminho é `rw [hs']` +
  `split` com `next hlt =>`.
- `omega` não resolve `2 ^ UScalarTy.U64.numBits` (expoente opaco):
  pinar `18446744073709551616` com `native_decide` e reescrever.
- `simp [UScalar.saturating_add]` estoura maxRecDepth: receita `show`
  com `Nat.min`/`Nat.mod_eq_of_lt` no nível Nat.
