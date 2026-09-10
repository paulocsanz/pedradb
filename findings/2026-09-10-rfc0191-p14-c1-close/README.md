# RFC-0191 P1.4 — C1 joint `model→close` (`c1_joint_election`)

Data: 2026-09-10. Commit: (este land).

## O que subiu

- `C1` no `scripts/ratchet/product_guarantees.tsv`: `model → close`,
  `floor_promoted` 3 → 4. Gate `product-floor` GREEN
  (D1=close R1=atom T1=atom C1=close, promoted=4≥4; selftest 4/4).
- Teorema de produto `c1_joint_election` (`formal/aeneas/lean/Membership.lean`):
  ∀ contagens (`old_yes old_n : U64`) + ∀ `Option (U64 × U64)` do joint,
  `joint_election_ok = ok (maioria C-old && (sem joint || maioria C-new))`
  contra a maioria pura `maj`. Não é rfl do corpo: o RHS é especificação
  fechada; o corpo só colapsa via `majority_of_closed`.
- `majority_of_closed`: `majority_of n = ok (maj n)` com `maj n =
  if n = 0 then 1 else mk (n.bv/2 + 1)` — abre o div/add checados da Aeneas
  com `UScalar.div_bv_spec` (y=2≠0) e `U64.add_bv_spec` (bound
  `n/2 + 1 ≤ 2^64−1` via `U64.lt_succ_max` + omega).
- Corolários (o texto do P1.4, ∀ contagens): `c1_old_majority_alone_refuses`
  (C-old majoritário + C-new minoritário ⇒ `ok false` durante joint) e
  `c1_both_majorities_elect` (ambas ⇒ `ok true`).
- Dente as-is ∀: `c1_as_is_elects_on_old_alone` — o mutante elege com C-old
  sozinho para qualquer `Option` do joint (RHS ignora `new_yes`).
- Dentes concretos N≤3 do harness (`joint_election_ok_needs_both`,
  as-is par (2,3,some(2,4))) continuam — não substituem o close.

## Fronteiras (o que NÃO é)

- Close de **produto**, não segundo close de catálogo: nenhuma linha nova em
  `close_proofs.tsv` — `joint_election` já é par extraído, e registá-lo
  droparia `proof_depth.extract` 276→275 < floor. `depth-floor` GREEN
  (extract=276, close=1, atom=1, data_fate=130).
- O `>=` do corpo extraído elabora como `decide (GE.ge …)` (coerção
  Prop→Bool); helpers `ge_true_of_not_lt`/`ge_false_of_lt` com ascrição
  `((x >= y) : Bool)` — sem ascrição o enunciado vira Prop
  (`(x ≥ y) = (true = true)`) e não paga.
- `sorry` no build: apenas prelude upstream da Aeneas (StringIter/Slice);
  `Membership.lean` tem zero.

## Provas

- `lake build Membership` verde.
- `cargo test -p pedradb-raft --lib joint_election_ok_on_live_old_only_is_not_ok` ok.
- Gates: `check_product_floor.py` (+`--selftest` 4/4), `check_depth_floor.py`,
  `check_ledger_consistency.py` (12 ponteiros) verdes.
