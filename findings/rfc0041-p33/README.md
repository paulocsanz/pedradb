# RFC-0041 — revert do catch-up 80 µs → 50 µs (sem remesura)

2026-08-18. Load ~79–110 em 12 CPUs. **Nenhuma remesura.** Mapa oficial
continua `findings/rfc0041-p11/head3/`.

## O que aconteceu

O p30 subiu `CATCHUP_WINDOW_DEFAULT` 50 → **80 µs** citando "80 µs ≪ um
`fdatasync`" — aritmética errada (fd ≈ 26 µs; 80 > 26) e **sem medição
quieta**. O único dado quieto permanece 50 µs → raftlog_mc4 1.792
(head3).

## Por que reverter

Break-even do wait fat-batch: cada membro já enfileirado paga a janela
inteira `W`; o grupo economiza no máximo `(k−1)` fds serializados. Ou
seja, `k·W ≤ (k−1)·fd` ⇒ `W ≤ (k−1)/k · fd < fd`. Janela acima de ~1 fd
(~26 µs nesta caixa) não pode se pagar por membro esperando. 80 µs
quebra o bound por ~3×.

`PEDRA_CATCHUP_US` continua existindo para sweep em caixa quieta
(25/50/80) quando a caixa permitir mediana 3× honesta.

## Landed

- `CATCHUP_WINDOW_DEFAULT` de volta a **50 µs**; comentário agora registra
  o bound e por que 80 foi revertido.
- `catchup_window_knob_roundtrip_and_latency_mode` volta a fixar 50 µs.

## Tests

`catchup_window_knob_roundtrip_and_latency_mode`, `catchup_bound_policy`
(1-op fd/2; 16-op janela cheia — política não mudou, só o default).
