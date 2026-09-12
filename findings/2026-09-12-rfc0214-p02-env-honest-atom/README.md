# RFC-0214 P0.2 6/6 (FECHAMENTO) — átomo `catalog:env_honest_sync`

Data: 2026-09-12. Sexta e última promoção de P0.2 (costura Env,
env_crash ×6). Escada viva: floor_atom 133→134,
floor_extract 145→144 (cap_data_fate 0<=0 imutável).

## O que foi pago

Teorema `honest_sync_fate_iff` em
`formal/aeneas/lean/EnvCrash.lean`: fate ∀ sobre o corpo extraído
— `(honest_sync_protects_all m cut = ok v) ↔ v = true`.

## Semântica

Após a barreira honesta (`sync m Honest = ok {m with synced :=
m.written}`), a janela legal colapsa num ponto: `written ≤ cut ≤
written` força `cut = written` (antissimetria) — todo crash legal
preserva o log INTEIRO. Pernas citadas: `sync_fate_iff` (3/6) e
`crash_legal_fate_iff` (1/6) desta fatia — a corolária só encadeia
os dois átomos registrados.

## AS-IS recusado

O twin as-is promove sync mentiroso (`sync_lying_promotes_as_is`)
— a barreira prometida não existe e o crash come o log.

## Planta DST

`crates/pedradb-sim/src/three_teeth_plants.rs`:
`env_crash_on_live_recording_is_not_ok` — verde (1 passed).
Kernel `env_crash_kernel.rs` intocado em toda a fatia (nenhum
re-extract necessário em P0.2).

## Cirurgia de catálogo

`promote_atom.py env_honest_sync`: linha `atom` no
`close_proofs.tsv` (entrada `honest_sync_protects_all`);
`atom_reason` datado no catálogo; floor_atom 133→134,
floor_extract 145→144; residuals/proof_depth re-carimbados no
mesmo commit.

## Escada (FECHAMENTO P0.2)

floor_atom 128→134, floor_extract 150→144 — meta da fatia batida
exata. Gate `check_depth_floor.py` GREEN: extract=144 (floor 144),
atom=134 (floor 134), close=6, data_fate=0<=0, residuals == live.

## Gates

`lake build EnvCrash` verde; planta DST verde;
`check_depth_floor.py` GREEN; 1 teorema novo neste commit
(`git show <c> -- …EnvCrash.lean | grep -c "^+theorem"` = 1).
6 promoções × 1 teorema = 6 commits (2977e49e, 44bed493, 49f8461e,
02dc3a64, 3bacca3d, este).
