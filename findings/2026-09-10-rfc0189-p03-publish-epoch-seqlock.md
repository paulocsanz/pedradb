# RFC-0189 P0.3 — publish sem RMW de epoch quando nenhum TLS de leitura existe — done

**Data:** 2026-09-10
**Peer / métrica:** corte de mecanismo no publish (0,82 µs/op atribuídos pelo P0.1);
efeito end-to-end medido no meter final do RFC.
**Veredito:** KEEP — mecanismo aterrado nos dois caminhos de publish rápido, testes ×3
verdes, famílias TLS/count verdes, pernas de leitura ycsb_c/deps_mvcc_latest sem regressão.

## O corte

Antes: todo publish no ramo rápido (caches de leitura vazios — shape só-escrita) pagava
DOIS `fetch_add(Release)` — `read_cache_epoch` + `point_tls_epoch` — para invalidar caches
que nenhuma thread lia (o TLS de leitura vive no compat: `LAST_GET`/`LAST_CF`/`LastCount`).

Agora: um contador `read_tls_fills` por `Db` (nunca reseta — recover é conservador).

- **Escritor** (`db.rs` `invalidate_read_answers` ramo rápido; `concurrent.rs`
  `apply_group_cold` do RFC-0190 — o publish do líder sem Db-lock): `load(Acquire) == 0`
  ⇒ pula os dois RMWs.
- **Leitor** (compat): `claim_read_tls_once()` — um `Cell<u64>` thread-local guarda a
  base da instância já reivindicada; na primeira vez faz `fetch_add(Release)` no contador.
  Claim ANTES do load de epoch que etiqueta a primeira resposta cacheada. O check por-get
  é um read de Cell + compare (custo ≪ hit de TLS). Claim apenas nos 4 sites de fill
  computado-por-leitor: `get`, `contains`, `get_cached`, `count_named`.
- **Put-warm NÃO claim**: a entrada warm carrega o epoch pós-próprio-put, então um bump
  concorrente nunca a protegeu (janela pré-existente, envelope idêntico sem o corte);
  não claim mantém o shape só-escrita do bench (`deps_cache_overwrite_mc4`, payload
  100 B ≤ 1024 aquece o TLS mas não claim) com `fills == 0` do início ao fim —
  todo publish da shape pula os dois RMWs.

## A validação seqlock do primeiro fill (por que é seguro)

Invariante: uma entrada TLS só serve resposta velha se algum publish que mudou a resposta
pulou o bump DEPOIS do fill. O handshake fecha isso em ambos os caminhos:

- Leitor: claim → load epoch → compute (sob `inner.read()` / `mem.read()`) → store.
- Escritor: write na memtable (seção serializada com o compute do leitor pelo RwLock do
  Db no caminho guardado, pelo RwLock do memtable no cold) → `load(fills)`.

Caso a caso: ou o compute do leitor terminou antes da seção do escritor (o claim dele
precede o load do escritor ⇒ `fills ≥ 1` ⇒ bump ⇒ a entrada com epoch pré-compute
auto-invalida), ou o compute começou depois da seção (viu o dado novo ⇒ entrada fresca).
`claim(Release)`/`load(Acquire)` + a cadeia happens-before do lock fecham a ordem. O
grupo `group_commit_kernel` já proíbe por teste de fonte publicar com o write-lock do Db
solto (o caminho guardado); o cold publíca sob o lock do memtable — mesma serialização
com o scan do leitor.

## Prova de que o skip engaja no caminho real

`publish_skips_epoch_rmws_until_read_tls_claimed` (pedradb-core, ×3 verde): puts reais
(`put_with`) — lone e pipeline (2 threads) — com ambos os epochs CONGELADOS; após
`claim_read_tls()`, o próximo put volta a bumpar.

## Monotonicidade concorrente (teste nomeado do RFC)

`tls_first_fill_is_monotonic_under_puts` (rocksdb-compat, ×3 verde): 2 threads — putter
com contador crescente (valores 1048B > 1024 ⇒ put-warm desligado ⇒ `fills == 0` durante
o stream) vs getter em loop. Valor observado nunca regride; após o último put (Ok ⇒
publicado), o get final vê o contador final — uma entrada presa por bump pulado
serviria abaixo do final para sempre.

## Não-regressão das shapes de leitura (ycsb_c / "deps_get" = deps_mvcc_latest)

Estrutural: com leitura presente, o primeiro fill claima e todo publish seguinte executa
exatamente a sequência de antes (mesmos dois fetch_add, mesma ordem); o caminho de
leitura (hit TLS) ganhou apenas um Cell::get + compare. O delta só existe no regime
`fills == 0`, onde não há leitura para regredir.

Pernas locais (Darwin, DIAG, 3× cada, mesmo schedule):

| shape | before (3×) | after (3×) |
|---|---|---|
| ycsb_c (zipf, 4096 keys) | PENDING | PENDING |
| deps_mvcc_latest | PENDING | PENDING |

(A preencher quando a árvore compilar de novo — sessão concorrente 0190 está
mid-edit no `memtable.rs` no momento desta linha; números vão no update.)

Família de regressão semântica: compat `tls` 11/11, `count_` 4/4 — os testes de
invalidação por put continuam verdes (o claim restaura o bump assim que alguém lê).

## O que NÃO mudou

- Slow path (caches preenchidos, fat reset, >32 dirty keys) intacto — bumps e
  `key_gen.touch` por chave (RFC-0154 P1.5) como antes.
- `read_tls_fills` nunca reseta (fence recovery conservador: bump a mais
  é seguro, bump a menos não).
- `pedradb-core` segue `#![forbid(unsafe_code)]`.
