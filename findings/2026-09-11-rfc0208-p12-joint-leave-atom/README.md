# RFC-0208 P1.2 (4/4) — atom `joint_leave`: o joint ainda ativo ⟺ fatos de ids diferentes

Data: 2026-09-11. Par `joint_leave` (`catalog:joint_leave`, kernel
`crates/pedradb-raft/src/membership_kernel.rs`, entry
`joint_still_active`, chamado ao vivo por `pending_joint_on` no
pedradb-store). Escada: cap_data_fate 87→86, floor_atom 44→45,
floor_extract 234→233, residuals atom 44→45 / extract 234→233 /
data_fate 87→86 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:joint_leave  joint_still_active_fate_iff
formal/aeneas/lean/Membership.lean  joint_still_active`). Com esta,
a cadência ×4 do P1.2 fecha nos números exatos do RFC: cap 90→86,
floor_atom 41→45, floor_extract 237→233.

## O que foi pago

O destino da configuração joint sobre TODOS os inputs
(Membership.lean, `joint_still_active_fate_iff`):

```lean
theorem joint_still_active_fate_iff :
    ∀ (old new : Slice U64) (v : Bool),
      (joint_still_active old new = ok v) ↔
        ((v = true ∧ old ≠ new)
          ∨ (v = false ∧ old = new))
```

O joint está ativo EXATAMENTE enquanto as fatias de ids antiga e
nova DIFEREM (igualdade elementwise U64): convergiu (old = new), o
joint saiu. O corpo extraído é `PartialEqShared.ne` sobre a
instância de slice gerada — SEM pure-lift direto; a prova atravessa
a ponte de specs da Aeneas: `PartialEqSlice.eq_homo_spec` (com o
`ne` escalar U64 como pure-lift via `WP.spec_ok`) dá o spec da
igualdade de fatias, `WP.spec_imp_exists` extrai `beq` com
`eq ... = ok beq ∧ (beq ↔ old = new)`, e o corpo do `ne.default`
(`ok (¬ (← eq ...))`) fecha por `rw` + casos. Primeira promoção do
repo que usa os spec-theorems da Aeneas como ponte semântica —
molde bancado para futuros corpos que atravessam traits.

O mutante AS-IS `ok false` declara todo joint morto na chegada —
sairia do joint antes da convergência (perde quórum duplo).

## Verificação

- `lake build Membership` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=45 (floor 45), extract=233 (floor 233),
  data_fate=86≤86, ledger 299/266/33.
- Planta DST `joint_still_active_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) — 1 passed.
