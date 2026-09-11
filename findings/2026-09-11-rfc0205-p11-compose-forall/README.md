# RFC-0205 P1.1 — composição ∀ off-lock (tier compose, sem registro)

Data: 2026-09-11. Commit: este. Arquivo: `formal/aeneas/lean/
ComposeConcurrent.lean` (importa `Flush` para compor com o close
registrado). Escada INALTERADA (compose não registra): close 6 /
atom 37 / extract 241 / cap_data_fate 94.

## O que foi pago

(a) `concurrent_publish_fate_forall` — o lado publish do protocolo
off-lock sobre TODOS os inputs: `∀ wal_io_ok, may_publish_group
wal_io_ok = ok wal_io_ok` (corolário do close registrado P0.1
`may_publish_group_ok_iff_wal_io_ok` sobre o outro lib).

(b) `wal_rotate_decision_fate_forall` — a regra de rotação sobre
TODOS os registros de pin; a disjunção exata veio do corpo extraído
(FlushKernel.lean:416), não da intuição:

- `RotateWal ⟺ mem_empty ∧ ¬imm_present ∧ ¬pin_live ∧
  ¬parked_unflushed ∧ ¬commit_inflight` (registro limpo nos 5 campos);
- `KeepWal ⟺ ¬(...)` — registro ocupado NUNCA trunca o WAL do writer.

Também o gêmeo as-is `wal_rotate_decision_as_is_ignore_pin_fate_forall`
: o mutante furado droppa `pin_live` da disjunção — PIN VIVO SOZINHO
deixa de segurar o WAL (a mentira que os twins DST cravam).

Ponte: `try_rotate_step_rotates_iff_all_clear_record` — instancia o
close REGISTRADO do Flush (`try_rotate_step_rotates_iff_pins_clear_
segment_live`, o passo do caller em db.rs) com a ∀ acima: o passo
dispara `rotate_wal_now` EXATAMENTE com registro limpo + sem recheck
inflight + segmento vivo. COMPOSIÇÃO, não duplicação: o registro
possui a direção decisão→passo; a ponte acrescenta registro→decisão.

(c) Os 4 dentes concretos existentes viraram COROLÁRIOS por
instanciação das ∀ (nomes mantidos: `concurrent_publish_and_inflight_
keep_wal`, `concurrent_publish_ok_and_idle_rotates`, `concurrent_as_is_
publish_lie_inflight_still_keeps`, `occ_snap_published_and_no_publish_
on_wal_fail`).

## Verificação

- `lake build ComposeConcurrent` verde; zero `sorry` no arquivo.
- `bash scripts/lean_extracts.sh --required` ok (61 libs + 15 compose).
- Gates 3× GREEN (depth-floor, product-floor, ledger) — escada
  intocada.
- Twins DST dirigindo a produção (pedradb-core): 
  `may_publish_group_on_live_group_is_not_ok` 1/1;
  `wal_segment_is_empty_on_live_zero_is_not_ok` 1/1.

## Por que NÃO vira linha do registro (close_proofs.tsv)

A regra do registro exige um par/entry ÚNICO do catálogo (kernel,
entry, as-is, planta) — a moldura de `check_depth_floor.py` conta
linhas por par singular. A composição off-lock atravessa DOIS kernels
(`group_commit_kernel` × `flush_kernel`) e três entradas
(`may_publish_group`, `wal_rotate_decision`, `wal_segment_is_empty`)
— não há par único para registrar. É o mesmo motivo dos outros libs
compose (12→15 hoje): o tier compose é ponte entre closes/atoms
registrados, não um degrau próprio. A ponte acima demonstra o uso
correto: componhe COM o registrado (`Flush`), nunca o duplica.
