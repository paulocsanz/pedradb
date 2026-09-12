# RFC-0214 P1.1 6/8 — átomo `catalog:write_ack_append`

Data: 2026-09-12. Sexta promoção de P1.1 (cqe ×5 + write_ack
×3). Escada viva: floor_atom 139→140, floor_extract 139→138.

## O que foi pago

Teorema `on_append_fate_iff` em
`formal/aeneas/lean/WriteAck.lean`: fate ∀ sobre o corpo
extraído — o ledger avança `written` para `w` iff a soma
checada `l.state.written + bytes = ok w` (append sem
overflow; fail/diverge de um lado casa com fail/diverge do
outro).

## Semântica

`WriteAckLedger.on_append` só promete bytes que a soma
checada do `WalState` aceitou — overflow trava o ledger, não
conta silenciosamente a menos. É o primeiro degrau do
contrato write→ack: todo byte ackado nasce de um append que
o kernel de admissão deixou passar.

## AS-IS recusado

O twin as-is `write_ack_ledger_as_is` acka o grupo sem
barreira — snapshot (64, 0, 64): o Ok promete durabilidade
que a classe não tem.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`verified_write_ack_on_live_profile_is_not_ok` — verde
(1 passed) APÓS a correção live deste commit. Kernel
`write_ack_kernel.rs` intocado nesta promoção (nenhum
re-extract necessário).

## Correção live (mesmo commit)

A planta, vermelha em toda bisseca desde o round-8, expôs
regressão real — não ambiental: o caminho lone G1
(`lone_commit` em `crates/pedradb-core/src/concurrent.rs`)
não avançava o ledger. Put sync de cliente único (a forma
principal do perfil verificado) deixava o ledger frio em
(0,0,0) — `acked > 0` nunca segurava. Corrigido espelhando
o caminho de grupo: append (delta de posição do WAL) →
barreira no fd Ok → ack + `assert_inv`; fence (Err) registra
o append mas nunca a barreira, igual à costura io_err do
grupo. Diagnóstico que fechou: o `put` single-client nunca
passa por `commit_ops_with` — a condição
`wal_sync_required(true, do_sync, false)` com perfil
verificado manda para `lone_commit`.

## Cirurgia de catálogo

`promote_atom.py write_ack_append`: linha `atom` no
`close_proofs.tsv` (entrada `on_append`); `atom_reason`
datado no catálogo; floor_atom 139→140, floor_extract
139→138; residuals/proof_depth re-carimbados no mesmo
commit.

## Escada

floor_atom 139→140, floor_extract 139→138. Gate
`check_depth_floor.py` GREEN: extract=138 (floor 138),
close=6 (floor 6), atom=140 (floor 140), count=7.

## Gates

`lake build WriteAck` verde; planta DST verde (1 passed,
testada isolada com exit checado ANTES da promoção);
`check_depth_floor.py` GREEN; 1 teorema novo neste commit.

## Baseline de regressão (registo datado)

Suítes completas rodadas antes do commit com a correção
aplicada: pedradb-core 917 passou / 23 falhou; pedradb-sim
71 passou / 2 falhou (`enospc_mid_flush_fences_transient`,
`r1_modelo_on_live_delete_shape_is_not_ok`). As 25 falhas
reproduzem em HEAD limpo (c57a78d1) SEM a correção e SEM a
cirurgia de catálogo (arquivos restaurados por `git checkout`
no teste de baseline) — vermelho pré-existente desta máquina,
não causado por este commit. Delta da correção: −1 falha
(a planta deste átomo, de vermelha a verde).
