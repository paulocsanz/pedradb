# RFC-0046 P2.6 — bloom sidecar: leitura abaixo do watermark, A/B medido

`examples/rfc0046_p26_bloom_ab.rs` — 2026-08-21, commit seguinte a `b7d4dff`.

## O caso que só o P2.6 poda

O stream de archive é ordenado por chave, então para leituras **pontuais**
o bound de key-coverage do manifesto (P2.5) já resolve a maioria dos casos:
os segmentos têm fatias de chave disjuntas dentro de uma mesma passada. O
caso que sobra é a **chave nunca escrita** (ou sem versão abaixo do floor)
cuja posição cai **dentro do coverage de todo segmento**: a prova de `None`
por spans de seq precisa então de todos os candidatos, e sem bloom cada um
é um CRC-walk completo.

Workload construído para isso: 8 192 chaves (`k00000..k08191`), cada rodada
escreve o keyspace inteiro **menos um buraco fixo de 16 chaves** (`k04089..k04104`);
o alvo `k04096` nunca é escrito. Uma passada de `compact_horizon()` por rodada
(3 s de pause > janela de 2 s) sela ~1 segmento por rodada — todo segmento tem
coverage `[k00000, k08191]` (o bound do P2.5 não poda nada) e nenhum contém o
alvo (bloom negativo em todos).

## Pernas

- **sidecar** — como selado (bloom + intervalos presentes).
- **walk** — sidecars removidos: sidecar ausente falha aberto, que é
  exatamente o comportamento pré-P2.6.

Intercaladas 3× para cancelar drift da caixa. **As duas pernas respondem
`None` em todas as leituras** — o A/B é também cross-check de correção do
caminho fail-open (mesmo veredito com e sem filtro).

## Números (stdout.txt, caixa SUJA load1≈41 início e fim)

| | valor |
|---|---|
| segmentos | 41 |
| archive | 24 548 251 B (23.4 MiB) |
| sidecars | 41 arquivos, 331 KiB (**1.4% do archive**) |
| snap.seq / earliest_readable | 8 176 / 138 977 (abaixo do watermark ✓) |
| sidecar | 39.5 / 51.6 / 71.1 µs/read (fases 0/1/2) |
| walk | 2 366.5 / 2 169.4 / 2 379.6 µs/read |
| **melhor vs melhor** | **39.5 vs 2 169.4 µs/read = 55×** (fases: 33.5–60×) |
| bytes por leitura | walk ≈ 23.4 MiB vs sidecar ≈ 331 KiB (72×) |

## Limitações

- Caixa suja (load 41 — sessão paralela rodando): os **absolutos** estão
  inflados nas duas pernas; a alegação é a **razão**, que se mantém 33–60×
  mesmo sob load 40. Absolutos de referência pedem caixa quieta.
- Chaves de overwrite (presentes em todo segmento) não ganham nada do
  bloom — positivo nunca poda; o ganho medido é especificamente a prova de
  `None`/ausência e leituras cuja chave não está naquele segmento.
- Espelho remoto segue walk (sem sidecar no objeto) — inalterado.
