# RFC-0219 P2.1 — recusas medidas por sítio (2026-09-13)

Fila `db.rs` medida no fim do P2.1 (grep datado abaixo): **19 linhas** =
15 sítios de código recusados + 4 linhas de comentário/doc (nunca foram
código). O P2.1 fecha com: 34 sítios resolvidos (18 pulls = 18 pares
nascidos átomo + 6 sítios drenados a `match` em kernels já pareados) e
**15 recusas nomeadas** — o número publicado que ajusta o alvo datado
do P2 (RFC-0219 «Meta mensurável»).

## Recusa por grupo (o porquê medido de cada um)

**R1 — `if let Err(e)` com binding usada (9 sítios: 7049, 7122, 8798,
10393, 10427, 10504, 10536, 10558, 11357).** O corpo propaga/retorna
`e`. A "decisão" é o desfecho da própria I/O (território Env — cânone:
Env não é provável); um plano enum sobre `is_ok()` apenas re-declararia
o `Result` sem mover nenhum data-fate para o kernel. Fósforo:
`persist_manifest_durable` ×2, `sync_dir_if_required`,
`vlog_prepare_wal` ×3, `ensure_not_fenced`, `wal_sync_group`,
`fsync_sst_paths`.

**R2 — `if let Some(..)` com binding usada (3 sítios: 2034, 3293,
6179).** O corpo consome o valor amarrado (origin escalada no recovery
F171; pin contado; pin tomado e usado). O `Option` não é um portão
data-fate — é dado fluindo; um enum sobre `is_some()` duplicaria o
match sem isolar decisão nenhuma.

**R3 — `if let Some((k, _))` de `last_visible_under_prefix[_with]`
(3 sítios: 4160, 4228, 4355).** A decisão de visibilidade vive DENTRO
do lookup (lógica por-chave de snapshot/tombstone sobre estado da
tabela — sprawl de estado). A sorte de visibilidade já tem kernel
próprio (`visible_at`, `point_cache_validity`, `point_tombstone`);
embrulhar o `Option` aqui é inflação de número no estilo
wrap-`is_empty` (anti-padrão nomeado pelo RFC).

## Linhas de comentário (4 — nunca código)

6213 (doc rotate), 8629 (comentário fence), 9425 (doc fence report),
10345 (doc group commit) — casam no grep por conterem as palavras, não
são portões.

## Alvo datado ajustado (número publicado)

- Original: P2 = 345/367 = 94,01% (75 pulls, todos par novo).
- Medido: 75 = 53 db.rs + 22 concurrent.rs; db.rs rendeu 18 pares
  (15 recusas + 6 drenos sem par + 13 sítios multi-portão de P1.1
  pagos por 3 pares) — a fila não é 1 sítio = 1 par.
- Ajuste: fim = (288 + P₂.₂)/(310 + P₂.₂) com P₂.₂ = pares nascidos
  no concurrent.rs ≤ 22 − R_c (R_c = recusas medidas lá). Teto sem
  recusa em concurrent.rs: 310/332 = **93,37%**. As 15 recusas db.rs
  já estão embutidas (os 18 pares em vez de 33).

## Captura do grep (2026-09-13, pós-drenos, HEAD aa48c203)

```
$ grep -n -E 'if .*(sync|flush|durable|visible|publish|fence|fsync)' crates/pedradb-core/src/db.rs
2034:                    if let Some(origin) = resync_origin {
3293:        if let Some(ref p) = self.flush_read_pin {
4160:                if let Some((k, _)) = table.last_visible_under_prefix(prefix, snapshot, hi) {
4228:            if let Some((k, _)) = table.last_visible_under_prefix(prefix, snapshot, None) {
4355:                if let Some((k, _)) = table.last_visible_under_prefix_with(
6179:        if let Some(pin) = self.flush_read_pin.take() {
6213:    /// Rotate WAL even if [`Self::flush_read_pin`] is live (pre-fix hole).
7049:            if let Err(e) = self.persist_manifest_durable() {
7122:                    if let Err(me) = self.persist_manifest_durable() {
8629:                // Leave prior handle in place if any; fence so puts stop.
8798:                    if let Err(e) = self.sync_dir_if_required(&self.dir) {
9425:    /// The first fence's report, if this Db was ever durability-fenced
10345:    /// Rocks-style group commit: many client batches, one fsync if any requires sync.
10393:                if let Err(e) = self.vlog_prepare_wal(g.any_sync) {
10427:                if let Err(e) = self.vlog_prepare_wal(g.any_sync) {
10504:        if let Err(e) = self.ensure_not_fenced() {
10536:        if let Err(e) = self.vlog_prepare_wal(g.needs_sync()) {
10558:                if let Err(e) = self.wal_sync_group() {
11357:        if let Err(e) = Self::fsync_sst_paths(&self.env, &self.dir, &paths, self.sync) {
```

Contador trampolim no fim do P2.1: db.rs 19 + concurrent.rs 22 = 41.
