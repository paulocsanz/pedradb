# RFC-0046 — disk sizing A/B: o caminho default não devolve disco ao horizonte (P0.5 fecha)

2026-08-21. Duas rodadas de medição — a primeira com um bug de workload
que a invalidou (erratum abaixo), a segunda correta e pareada. Load
irrelevante (contagem de bytes, não qps). Exemplo: `cargo run -q
--release -p pedradb-core --example rfc0046_sizing_ab` (perfis via
`SIZING_PROFILE=all|window|trigger`).

Workload (correto): 256 chaves × 1 KiB incompressível **estáveis entre
rounds**, 40 rounds de overwrite completo, flush por round, pausa de 3 s
a cada 4 rounds (versões cruzam o horizonte curto de 2 s no meio da
corrida). Live set = 256 KiB; escrito = 10 MiB. Janela mecânica (2 s +
cap 4 MiB) como proxy do default de produto (24 h + 1 GiB).

## ERRATUM — a primeira rodada (96bbb71) media um workload sem overwrite

O exemplo regenerava o sufixo aleatório da chave **a cada round**
(`kbuf.extend_from_slice(&key[..2])` com `key` re-sortido por (k, round)),
então cada round escrevia 256 chaves *distintas*: zero overwrites, zero
versões repetidas, **nada para o GC dropar em perfil algum**. A
identidade byte-a-byte window≡all dessa rodada era artefato (todo
key-set tinha 1 versão por chave; `gc_compact_entries` corretamente não
derruba nada). Os números 82–95× "live" também eram calculados contra um
live set falso de 256 KiB — o live real era os 10 MiB de chaves distintas. A
inferência de mecanismo (floor atrasa, níveis velhos nunca reescritos),
porém, era correta — e é o que a rodada correta agora **confirma com
evidência válida**. Artefatos antigos preservados em `invalid-workload/`.

## Números (workload corrigido)

Sem o trigger P0.5 (`stdout-no-p05.txt`, HEAD do exemplo corrigido com
db.rs do trigger removido via stash):

| perfil | db (LSM) | history/ | total | total/escrito |
|---|---:|---:|---:|---:|
| `all` (F20, pré-0046) | 10 991 231 B | 0 | 11,0 MB | 1,05× |
| `window` (2 s + cap) | **10 991 231 B** | 1 043 686 B | 12,0 MB | 1,15× |

**O LSM do perfil window é byte-idêntico ao F20** (10 991 231 B em ambos)
E o archive adiciona uma cópia por cima — o window fica estritamente pior
que F20 em disco, exatamente como a rodada invalidada concluía (pelo
motivo errado). O watermark avança a cada compactação sem nada devolver.

Com o trigger P0.5 (`stdout-p05.txt`, mesmo binário + trigger):

| perfil | db (LSM) | history/ | total | total/escrito |
|---|---:|---:|---:|---:|
| `window` (2 s + cap) | **2 710 428 B** | 4 036 766 B | 6,7 MB | 0,64× |
| `trigger` (window + sst_count=8) | 2 710 428 B | 4 036 766 B | 6,7 MB | 0,64× |

LSM 4,1× menor que sem o trigger; o archive satura no cap (4,0 MB ≈ cap
4 MiB — abaixo do floor, cap decide). Total = live + janela em trânsito +
cap: a promessa do P0.2 passa a valer para o **total**, não só o archive/.

## Mecanismo (código confirmado; a rodada correta valida)

1. **O floor do horizonte sempre atrasa os inputs.** `maybe_auto_compact`
   com `l0_hit` mescla só os L0s novos (`compact_l0_into_l1`: "L0 → one
   new L1. Do not absorb the existing L1"). Versões cruzam o horizonte
   DEPOIS de chegarem a L1 — quando envelhecem, não estão mais nos
   inputs. Assimetria estrutural vs `auto_reclaim`: o floor do reclaim
   (`last_seq`) sempre excede o batch fresco (drop imediato — por isso o
   perfil 0047 limita disco); o floor do horizonte sempre atrasa.
2. **Nenhum caminho reescreve níveis velhos.** `compact_with_ssts_only`
   promove UM nível por vez (menor primeiro — L0 vence); a reescrita
   total só existia quando TUDO já está no `MAX_LSM_LEVEL`.
   `count_hit`/`bytes_hit` default `None`; `l0_hit` tem precedência.
3. **O archive não devolve disco do LSM** — só copia o que vai sair.

Wart adicional (disponibilidade, não silêncio): o watermark avança pelo
floor *reportado* (`note_version_gc_watermark`), não pelo drop *efetivo*
— com P2.1, leituras abaixo do watermark vão ao archive mesmo com a
versão ainda no LSM; se o cap derrubou o segmento arquivado, a leitura
falha `SnapshotTooOld` com a versão presente no LSM. Fail-closed, não
errado.

## Fix (P0.5, implementado)

Dead-weight-doubling trigger em `maybe_auto_compact`: quando o floor do
horizonte avançar além do último full reclaim E os bytes totais de SST
pelo menos dobrarem desde então, reescrever TODOS os SSTs com o GC floor
(archive-first, fail-closed em erro de archive — o trigger retenta no
próximo flush). Máximo uma reescrita por dobramento de peso morto
(gatilho clássico de size-ratio); estado in-memory
`last_horizon_reclaim: Option<(floor, bytes)>` — reopen pode pagar uma
reescreita extra (auto-limitante). `auto_reclaim` segue pelo caminho
dele (floor maximal, sem mudança).

## Raw

- `stdout-no-p05.txt` — workload corrigido, sem o trigger (contrafactual).
- `stdout-p05.txt` — workload corrigido, com o trigger (all/window/trigger).
- `invalid-workload/` — artefatos da rodada invalidada (bug do sufixo).
