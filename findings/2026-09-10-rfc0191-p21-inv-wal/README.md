# RFC-0191 P2.1 — Inv-WAL preservação num passo + corolário D1

Data: 2026-09-10. Commit: (este land).

## O que subiu

- `formal/aeneas/lean/WalState.lean` (agora importa `WriteAdmission`):
  - `wal_inv_closed` ∀ s: `inv_wal s = ok ((acked <= synced) &&
    (synced <= written))` — as duas contenções como conjunção Booleana
    (`synced ≤ written` é exatamente "synced está no
    prefixo-recuperável": o teto de um crash legal é `written`).
  - `wal_append_closed` ∀ s n w: um add ok ⇒ o append devolve
    `{s with written := w}` (barreira e acked não se movem).
  - `wal_append_preserves_inv_wal` ∀ s s' n: `inv_wal s = ok true →
    wal_append s n = ok s' → inv_wal s' = ok true` — o passo indutivo.
    Os casos `fail`/`div` do add checado contradizem `= ok s'`
    (`bind_fail`); o caso ok extrai `w.val = written.val + n.val` de
    `UScalar.add_equiv` e fecha com omega.
  - Corolário `d1_plan_append_preserves_inv_wal`: hipótese
    `wal_commit_plan need_sync sync_fail = ok AppendSyncApplyOk`;
    prova `rw [d1_wal_commit_plan] at hplan` (cita o close P1.2) e
    `exact wal_append_preserves_inv_wal …` (cita o lema). Os 3 casos de
    plano fora do caminho honesto (ApplyOk sem sync / Fence) fecham por
    contradição com `hplan` — a citação não é vacua.
- Linha de produto D1 **não** mudou (permanece `close`); nenhum TSV de
  profundidade tocado.

## Fronteiras (o que NÃO é)

- Um passo do append do kernel de geometria de prefixo — não é o
  `wal/writer.rs` de produção inteiro, nem ∀ interleavings de grupo.
- O corolário conecta o plano (que o rustc liga) ao invariant; a
  barreira física (fdatasync) segue sendo fronteira nomeada (0078).
- `<=`/`>=` extraídos elaboram como `decide (LE.le/GE.ge …)` — mesmas
  pontes do P1.4 (`decide_eq_true`, `UScalar.le_equiv`).

## Provas

- `lake build WalState` verde (WriteAdmission recompilado junto).
- Gates no commit: product-floor GREEN (D1=close …promoted=4≥4),
  depth-floor GREEN (extract=276 intacto), ledger GREEN (12 ponteiros),
  `lean_extracts.sh --required` ok.
