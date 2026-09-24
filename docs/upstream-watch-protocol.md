# Protocolo de watch upstream — Aeneas / Charon

**Status:** living (procedimento medido; cada re-pin atualiza este doc no mesmo commit)
**ID:** watch-0187
**Parents:** [0187](rfc/0187-teorema-experimento-tcb.md), fire 803 (`findings/2026-09-09-aeneas-iterator-widen.md`)
**Gate relacionado:** ledger (`scripts/check_ledger_consistency.py`) — pins são TCB nomeado

## A regra (permanente)

Re-pin de Aeneas/Charon/Lean **só** com *widen medido sem sorry*: após o
delta upstream, o conjunto traduzido do MESMO corpus tem que (a) crescer
— funções/formas que hoje bloqueiam passam a traduzir — E (b) nenhum
`sorry` novo aparecer nas extrações existentes. Fechar um bug upstream
não basta; travar mais também não re-pin (registra o negativo medido).
O re-pin é um commit único: pins + catálogo + gates verdes juntos.

Pins atuais (termos de prova do produto — o TCB do ledger):

| Ferramenta | Pin | Onde |
|---|---|---|
| Aeneas | `daa85d7` | extração (`$AENEAS` override) |
| Charon | `0.1.232` / `340b1af` | extração (`$CHARON` override) |
| Lean | `4.31.0` | `lake` build das extrações |
| Verus | `0.2026.08.09.92f466f` | `.github/workflows/proof-check.yml` |
| Kani | sha256-pinned | `.github/workflows/proof-check.yml` |

## Procedimento sandbox (medido, fire 803)

Cada rodada de watch segue exatamente estes passos (todos medidos em
2026-09-09/10; desvios precisam de registro em `findings/`):

1. **Worktrees**: Aeneas em `/tmp/aeneas-new` @ head upstream (89
   commits à frente do pin na medição do fire 803; head `505b6ca`),
   Charon @ `b104e24`. No macOS usar `gmake`; OCaml no PATH:
   `export PATH="$HOME/.opam/5.3.0/bin:$PATH"`.
2. **Build do Charon**: `gmake setup-charon` falha (nix "unsupported
   system") — usar `cargo build --bins` manual + copiar os bins para
   `../bin/`. Invocar SEMPRE via `charon cargo` (o `charon-driver`
   standalone morre com dyld error).
3. **Shim do kernel**: crate-shim com `#[path]` ABSOLUTO apontando para
   o fonte de produção (clone do kernel, nunca wrap-factory —
   wrap-factory é proibido pelo contrato three-teeth).
4. **`--sysroot default` é obrigatório**: sem ele, o sysroot miri
   compartilhado stale envenena as deps (~626 erros de tipos falsos).
5. **Veredito do delta**: parse do `.llbc` via JSON com guards para
   `None` (o llbc é single-line; `grep -c` subconta — usar
   `grep -o | wc -l` ou Python). Comparar contra a extração do pin
   atual: (a) conjunto traduzido cresceu? (b) `sorry` novo? Aplicar a
   regra acima.
6. **Registro**: afirmativo → re-pin num commit único + este doc;
   negativo → `findings/YYYY-MM-DD-*.md` com os números (bloqueios
   exatos, tamanho do llbc, contagens).

## Bloqueios medidos (estado fire 803 — referência para o próximo watch)

- `Box<dyn Iterator>` (`LayerStream` em `merge.rs`): Aeneas recusa no
  NÍVEL DE TIPO — "Dynamic trait types are not supported yet". O bloqueio
  é 100% lado Aeneas: a tradução charon do crate INTEIRO succeeds
  (~8.8 MB de llbc). O watch re-testa este sítio a cada delta upstream:
  enquanto o dyn-Trait não abrir, o heap-sift (`StreamingVisibleIter`)
  não vira kernel Aeneas — o caminho formal dele é Isolated-method
  (RFC-0187 P1.3) que remove o `dyn` do kernel.
- **Repro mínima + voz upstream (RFC-0188 P2.4):**
  `formal/aeneas/repro/dyn-iterator/run.sh` reproduz o bloqueio no pin
  atual com um crate de 2 structs (mesmo crate, mesmo pin): o controle
  (`Vec<u8>`) extrai; o repro (struct com campo `Box<dyn Iterator>`)
  recusa com a mensagem exata. Exit 0 = bloqueio presente (watch
  armado); exit 2 = upstream abriu dyn-Trait (rodar o procedimento de
  watch completo). Issue upstream com a repro e a pergunta de tracking:
  [AeneasVerif/aeneas#1343](https://github.com/AeneasVerif/aeneas/issues/1343)
  (2026-09-10; charon#123 já fechado como feito — o consumo do
  `TyKind::Dyn` pelo backend Lean é o que falta).
- `-filter-trait-methods` (`f9a8e33` upstream): testado no fire 803 —
  NÃO alarga o conjunto traduzido para o nosso corpus (negativo medido,
  sem re-pin).
