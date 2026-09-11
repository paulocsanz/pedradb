# RFC-0208 P0.1 — atom `grant_after_persist`: o destino do voto sob persistência durável

Data: 2026-09-11. Par `grant_persist` (`catalog:grant_persist`,
kernel `crates/pedradb-raft/src/vote_kernel.rs`, entry
`grant_after_persist`, chamado ao vivo por `on_request_vote`
pedradb-store). Escada: cap_data_fate 92→91, floor_atom 39→40,
floor_extract 239→238, residuals atom 39→40 / extract 239→238 /
data_fate 92→91 — tudo no MESMO commit, com a linha do registro
(`atom  catalog:grant_persist  grant_after_persist_fate_iff
formal/aeneas/lean/Vote.lean  grant_after_persist`).

## O que foi pago

O destino do voto sob o resultado durável da persistência sobre
TODOS os inputs (Vote.lean, `grant_after_persist_fate_iff`) — o
corpo é match duplo (decision × persist):

```lean
theorem grant_after_persist_fate_iff :
    ∀ (decision : VoteDecision) (persist : PersistOutcome) (v : Bool),
      (grant_after_persist decision persist = ok v) ↔
        ((v = true ∧ decision = .WouldGrant ∧ persist = .Ok)
          ∨ (v = false ∧ ¬(decision = .WouldGrant ∧ persist = .Ok)))
```

Voto concedido EXATAMENTE quando a decisão é WouldGrant E a
persistência durável deu Ok: Deny nunca concede, e WouldGrant com
persistência falhada também não — o caminho da eleição nunca entrega
um voto que não tornou durável. O mutante AS-IS ignora o resultado
da persistência.

Este é o par 1/8 do seam store/raft (RFC-0208): completa o kernel de
voto (o `vote_decision` do 0205 P0.2 decidiu; este decide o efeito
da durabilidade sobre a concessão).

## Verificação

- `lake build Vote` verde; zero `sorry` no arquivo.
- Gates 3× GREEN: atom=40 (floor 40), extract=238 (floor 238),
  data_fate=91≤91, ledger 299/266/33.
- Planta DST `grant_after_persist_on_live_queued_is_not_ok`
  (pedradb-store/three_teeth_queued) — resultado no commit.
- `bash scripts/lean_extracts.sh --required` ok.
