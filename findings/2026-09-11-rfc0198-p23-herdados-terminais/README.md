# RFC-0198 P2.3 — herdados do 0187 seguem terminais

Fatia de registro (docs-only): o RFC-0198 NÃO re-abre os três gates
herdados do 0187; o estado terminal registrado pelo RFC-0191 P2.4
continua valendo. Este commit apenas cita o registro existente e fecha
o checkbox.

## O que está citado (sem edição de ledger)

`docs/verification-ledger.md` §"Herdados do 0187 — estado terminal
(RFC-0191 P2.4)", commitado no HEAD deste commit:

- **Série L28** (33 pares `l28_*`, tier campaign): **user-gated** —
  promoção a teorema three-teeth exige decisão registrada (rank H).
- **Nightly experimental de durabilidade física** (TCG guest power-cut
  por barreira + `F_FULLFSYNC` no macOS): permanece nightly, sempre
  experimento — "persistiu no disco" além da barreira de SO não vira
  claim de teorema; a barreira de SO é TCB.
- **Exaustivo N=4 com poda/simetria**: aberto no 0187 por custo do
  runner.

Nenhum dos três foi re-aberto, re-fatado ou reescrito; alargar a
fronteira de qualquer um continua sendo movimento de ledger, não de
RFC. O arquivo `verification-ledger.md` NÃO foi editido neste commit
(está com hunk NÃO-commitado da sessão paralela — seção escada `count`
do RFC-0199 — e a regra do goal é nunca tocar o voo dela).

## Verificação (mesmo commit)

- Fatia docs-only: nenhum gate de escada envolvido; depth/product/
  ledger GREEN no commit (sem mudança de registro).
