# RFC-0213 P2.1 1/2 — atom `catalog:wal_recover` (`recover_collect_act_fate_iff`, WalRecover.lean)

Data: 2026-09-12. Par `wal_recover` promovido a `atom` (sem linha
close prévia — twin_kind close com extrato, não contava como
residual close), teorema `recover_collect_act_fate_iff` em
`formal/aeneas/lean/WalRecover.lean` sobre o extrato Aeneas de
`crates/pedradb-core/src/wal/recover_kernel.rs` (entry do catálogo:
`recover_collect_act`).

## LIBS — o buraco pré-existente fechado

`WalRecover` inscrito no LIBS do `scripts/lean_extracts.sh` NESTA
fatia (linha `EnvCrash WalState WalRecover …`): antes disso o
wrapper era construído pelo `lake build` do pacote mas o script
oficial de extratos não o exigia — o `--required` passava sem ele.
Extrato re-carimbado: `bash scripts/aeneas_wal_recover.sh`
(charon+aeneas no PATH) regenerou `WalRecoverKernel.lean`
**byte-idêntico** (fonte do kernel intocada desde o último carimbo;
`git status formal/aeneas/out/` vazio) — verificado, não é
assunção.

## O que o teorema diz (fate forall sobre o corpo extraído)

`recover_collect_act kind prefix_n can_skip skips in_resync = ok v`
↔ disjunção exata dos nove braços do match extraído: Record→kept,
CleanEof→stop, trio torn (Truncated/LengthCorrupt/UnknownType)→
fail-stop em prefixo vazio / keep-prefix vivo / resync sob o budget
∃i (`MAX_CONSECUTIVE_SKIPS = ok i ∧ skips > i` decide fail-stop vs
resync), OrphanFragment→fail-stop, Crc/ZeroHeaderTail→(mid-walk:
resync/keep-prefix/fail-stop por prefixo; alinhamento fresco:
fail-stop), Other→fail-stop. O `let i ← MAX_CONSECUTIVE_SKIPS`
extraído vira o ∃ medido no RHS — o molde ∃-chain.

## Por que data_fate não é mais necessário

Todos os nove caminhos do match cobertos pelo iff — o fate do
recovery é a estrutura do framing, não folclore. O as-is
(`recover_collect_act_as_is`) chama torn/length/zero-header de
CleanEof (perda silenciosa do prefixo acked) e resynca CRC ruim —
refutado ao vivo:
`recover_collect_act_on_live_exploded_crc_is_not_ok` (1 passed,
`cargo test --lib --manifest-path crates/pedradb-sim/Cargo.toml`).

## Ratchet

- `close_proofs.tsv`: +1 `atom catalog:wal_recover`
  (`recover_collect_act_fate_iff`, entry `recover_collect_act`).
- `proof_depth.tsv`: floor_atom 121→122, floor_extract 157→156,
  cap_data_fate 2→1 (floor_close 6 inalterado; residual close fica
  5 — o par não tinha linha close e seu kernel TEM extrato, logo
  nunca contou como close residual).
- `catalog.json`: `data_fate` removido, `atom_reason` datado.
- `residuals.json`: atom+1, extract−1, glue.data_fate−1.
- Gate `check_depth_floor.py`: GREEN (extract=156, ladder close=6
  linhas, atom=122, residuals close=5/atom=122 == live 5/122,
  count=7, data_fate=1).

## Lições (para o 2/2 e futuros matches de 9 braços)

- `split at hval` sobre o corpo do match extraído parte o MATCH em
  N braços com `heq : ctor = ctor-arm` — braços errados morrem por
  `absurd heq (fun h => C.noConfusion h)`; **nunca** `by simp`
  dentro de `first` (a falha do tactic-in-term não é recuperável e
  vaza `⊢ False`).
- Igualdades construtor=construtor NÃO aceitam padrão `rfl` no
  rintro/rcases (não são `x = t`) — bind por nome e `absurd`/ignore.
- Índices de disjunção aninhada: D_k precisa de k−1 `Or.inr`
  (errei D3/D5/D6 por um na primeira passada — conferir contando).
- `rintro` com padrão plano de 6 alternativas quebra linha APÓS o
  4º elemento parseia errado no nosso caso (Other) — o stepwise
  `rcases h with h1 | h1` é à prova de aridade.
- `exact M ⟨?_, …⟩` deixa metas não-sintetizáveis (`rfl` no `?_`);
  `refine M ⟨?_, …⟩` + bullet `first | exact Or.inl rfl | …`
  sintetiza certo.
- `subst h.2` não é sintaxe válida (subst quer variável) —
  `rcases h with ⟨_, hv⟩; subst hv`.
