# RFC-0191 P1.6 — sweep final: gates do pacote verdes, vermelhos herdados nomeados

Última fatia do RFC-0191. O sweep roda o escopo que a fatia define
(Lean/depth/product) mais os gates que o pacote tocou (ledger, seam).

## Verde (escopo P1.6, working tree @ cb09bf18+d6a223cb)

| verificação | resultado |
|---|---|
| `scripts/lean_extracts.sh --required` | ok — 61 libs + 5 compose, build completo |
| `lake build` dos 16 módulos com prova registrada (`close_proofs.tsv`) | verde; 0 `sorry` nos wrappers (grep em todos `formal/aeneas/lean/*.lean`: nenhum arquivo tem sorry) |
| `check_depth_floor.py` | GREEN — extract=248 (floor 248), close=1/1, atom=31/31, residuals == live, df 100≤100 |
| `check_product_floor.py` | GREEN — D1=close R1=atom T1=atom C1=close, promoted 4≥4 |
| `check_ledger_consistency.py` | GREEN — marker 298/262→265 recount pago no P2.4 |
| `check_seam_inventory.py` | OK — 15/15 sítios L2 |

## Vermelho herdado, nomeado (nenhum desta campanha; nada escondido)

1. `check_barrier_floor.py` RED: 3 sítios novos não-injetados em
   `crates/pedradb-core/src/concurrent.rs` (sync_all 2/1, sync_data 3/2,
   sync_dir 4/2). O arquivo está **não-commitado** (MM no git status) —
   trabalho da sessão otimizar paralela; pinar os sítios agora seria
   editar o ratchet no meio do voo dela. A régua é dela pagar o
   `barrier_sites.tsv` no commit dela.
2. `check_no_prod_time_spawn.py`: 96 violações, **todas** em
   `crates/pedradb-store/src/three_teeth_queued.rs` (wall-clock
   `SystemTime::now` no harness DST). Último commit do arquivo:
   8cd59249 ("fire 800", outra sessão); working tree limpo nele.
3. `pedra_formal.py`: exit 1 com 153 FAIL — todos herdados. Conjunto
   verificado **idêntico** entre os fires 787 e 788 (comm vazio); todo
   delta vs o baseline 786 é da sessão paralela (surface do
   `ratio_curve_kernel.rs` + drift handler_loc/kernel_loc).

## Fecho do pacote

Com P1.6 done, o RFC-0191 fecha **todas** as fatias P0/P1/P2:
- P0.1–P0.3 (ratchet de produto, corolário R1, ledger)
- P1.1–P1.6 (R1-value atom, D1 close, T1 atom, C1 close, trampolim P1.5,
  sweep)
- P2.1–P2.4 (Inv-WAL, Inv-LSM, cap 130→100 com 30 passos um-por-commit,
  registro terminal dos herdados do 0187)

Estado terminal da escada: extract 248 / close 2 / atom 31 / model 17;
cap_data_fate 100; floor_atom 31 (meta ≥8 superada 3,9×).
