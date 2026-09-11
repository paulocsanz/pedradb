# RFC-0208 P0.2 — atom `propose_ack_ok`: o destino do ack de propose (seam raft 2/2)

Data: 2026-09-11. Par `commit_raft` (`catalog:commit_raft`, kernel
`crates/pedradb-raft/src/commit_kernel.rs`, entry `propose_ack_ok`,
chamado ao vivo por `broadcast_append_after_propose`
pedradb-store). Escada: cap_data_fate 91→90, floor_atom 40→41,
floor_extract 238→237, residuals atom 40→41 / extract 238→237 /
data_fate 91→90 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:commit_raft  propose_ack_ok_fate_iff
formal/aeneas/lean/Commit.lean  propose_ack_ok`).

## O que foi pago

O destino do ack sobre TODOS os inputs (Commit.lean,
`propose_ack_ok_fate_iff`) — pure-lift `ok (commit_index >= index)`
(`>=` U64 é Prop-valued, corpo = `decide (commit_index >= index)`;
mesma lição do 0205 P1.2):

```lean
theorem propose_ack_ok_fate_iff :
    ∀ (index : U64) (commit_index : U64) (v : Bool),
      (propose_ack_ok index commit_index = ok v) ↔
        ((v = true ∧ commit_index >= index)
          ∨ (v = false ∧ ¬(commit_index >= index)))
```

Propose recebe ack EXATAMENTE quando o índice da entrada já está
commitado (index ≤ commit_index): sem ack fantasma para índice não
commitado; commitado nunca fica sem ack. O mutante AS-IS `ok true`
ack tudo.

## Claim datada do seam raft (contagem mecânica)

Após P0.1+P0.2, `vote_kernel.rs` e `commit_kernel.rs` têm ZERO
pares `data_fate` pendentes no catálogo (contagem executada na
cirurgia: `data_fate pendente por kernel raft:
{membership_kernel.rs: 26}` — os dois kernels singleton do raft
fecharam; os destinos de recovery do membership
`recover_must_apply`/`recover_drop_orphan_seg` já eram atoms do
0205 P1.2; os `*_node_counts` e o resto do membership seguem para a
cadência P1.2). Nota honesta: o texto original do slice dizia "os
TRÊS kernels raft" — o mensurável hoje é vote+commit fechados;
membership tem 26 pendentes nomeados para a cadência.

## Verificação

- `lake build Commit` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=41 (floor 41), extract=237 (floor 237),
  data_fate=90≤90, ledger 299/266/33.
- Planta DST `propose_ack_ok_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) 1 passed; 0 failed.
- `bash scripts/lean_extracts.sh --required` ok (61 libs + 18
  compose).
