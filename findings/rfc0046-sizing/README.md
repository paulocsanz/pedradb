# RFC-0046 — disk sizing A/B: a promessa "live set + janela + cap" é FALSIFICADA no caminho default

2026-08-21, 21:2x–21:3x. Load ~20 durante a janela (irrelevante: contagem
de bytes, não qps). Exemplo: `cargo run -q --release -p pedradb-core
--example rfc0046_sizing_ab` (perfis via `SIZING_PROFILE=all|window|trigger`).

Workload pareado: 256 chaves × 1 KiB incompressível, 40 rounds de overwrite
completo, flush por round, pausa de 3 s a cada 4 rounds (versões cruzam o
horizonte curto de 2 s no meio da corrida). Live set = 256 KiB; escrito =
10 MiB. Janela mecânica (2 s + cap 4 MiB) como proxy do default de produto
(24 h + 1 GiB) — mesmo mecanismo, constantes de bench.

## Números

| perfil | db (LSM) | history/ | total | total/live | total/escrito |
|---|---:|---:|---:|---:|---:|
| `all` (F20, pré-0046) | 21 535 931 B | 0 | 21,5 MB | 82,2× | 2,05× |
| `window` (2 s + cap) | **21 535 931 B** | 1 043 686 B | 22,6 MB | 86,1× | 2,15× |
| `trigger` (window + sst_count=8) | 21 533 053 B | 3 467 494 B | 25,0 MB | 95,4× | 2,38× |

**O LSM do perfil window é byte-idêntico ao F20** (21 535 931 B), e o
watermark avança a cada compactação (round 7: `earliest=993`; round 39:
`9185` de 10 240 seqs) — o floor marcha, nada é devolvido ao disco. O
perfil window fica **estritamente pior** que F20: retém tudo E arquiva uma
cópia. O gatilho `auto_compact_sst_count` (leva suposta de "full rewrite")
não só não limita como infla o archive (3,5 MB — re-archive de versões que
a compactação não devolve).

## Mecanismo (código confirmado, não teoria)

1. **O floor do horizonte sempre atrasa os inputs.** `maybe_auto_compact`
   com `l0_hit` mescla só os L0s novos (`compact_l0_into_l1`: "L0 → one new
   L1. Do not absorb the existing L1"). Versões cruzam o horizonte DEPOIS
   de chegarem a L1 — quando envelhecem, não estão mais nos inputs. O floor
   cobre só o que já saiu. Assimetria estrutural vs `auto_reclaim`: o floor
   do reclaim (`last_seq`) sempre excede tudo no batch (drop imediato — por
   isso o perfil 0047 limita disco); o floor do horizonte sempre atrasa.
2. **Nenhum caminho reescreve níveis velhos.** `compact_with_ssts_only`
   promove UM nível por vez (menor primeiro — L0 vence de novo); a cláusula
   de reescrita total só existe quando TUDO já está no `MAX_LSM_LEVEL`.
   `count_hit`/`bytes_hit` default `None`; e `l0_hit` tem precedência no
   `else if`.
3. **Não existe API pública de full-compaction horizon-aware.** `compact()`
   = sem GC; `compact_reclaim()` = floor de pin/last_seq (dropa a janela
   inteira, não o horizonte); `auto_gc_floor` é privado.

Wart adicional (disponibilidade, não silêncio): o watermark avança pelo
floor *reportado* (`note_version_gc_watermark`), não pelo drop *efetivo* —
com P2.1, leituras abaixo do watermark vão ao archive mesmo com a versão
ainda no LSM; se o cap derrubar o segmento arquivado, a leitura falha
`SnapshotTooOld` com a versão presente no LSM. Fail-closed, não errado —
mas é perda de disponibilidade por causa de um floor que não dropou nada.

## O que limita disco hoje

Só `auto_reclaim` (perfil compat/Rocks, RFC-0047: 9,4× menor que F20) —
porque o floor dele é sempre maximal nos inputs frescos. O retention
window do kernel, como entregue, **não limita o LSM** em workload algum em
que versões sobrevivam à residência em L0 (o caso normal; o default 24 h
garante que sim). A promessa "disco ≈ live set + janela + cap" do P0.2
vale só para o archive/, não para o total.

## Fix necessário (P0.5 proposto)

Rewrite de níveis velhos dirigido pelo horizonte: quando o floor avançar
além da versão mais antiga de um nível (ou por margem material — ex. floor
> último reclaim + fração do live set), reescrever esse nível com o GC
floor (absorver L1 na mescla L0 quando coberto, ou compactar o nível
velho diretamente). Custo controlado por margem, não por round. Até lá:
`docs/usage.md` documenta o estado real; colunas oficiais não mudam (24 h
em bench de minutos não envelhece nada — comportamento = pré-0046).

## Raw

- `stdout.txt` (A/B original), `window-debug.txt` (per-round watermark ×
  sst_bytes do perfil window), `trigger.txt`.
