# RFC-0200 P1.2 — ponte sift_step↔newest-first (camada tagged)

Commit: (este slice) `Merge.lean` + flips do RFC-0200 neste README.

## O que landou

Sete definições/teoremas no bloco `### RFC-0200 P1.2` de
`formal/aeneas/lean/Merge.lean` (sorry 0):

| Nome | Papel |
|---|---|
| `TaggedSift` | passo de sift com as 3 entradas bool do extract + a decisão `s : merge.SiftStep` |
| `tagged_kernel_decision` | predicado: `s` É a decisão do kernel (`sift_step r l b = ok s`) — não um valor arbitrário |
| `tagged_step_stays_iff_no_repair` | Stay ⟺ sem reparo — re-export DIRETO do close REGISTRADO `merge_sift_step_repairs_iff` (0188 P0.2): `rw` + `injection`, zero re-prova |
| `merge_step_newest_first_congr` | a premissa estrutural da cadeia é LOCAL ao par `(newer, older)` — não lê `kind`/`range_hidden` |
| `tagged_stay_preserves_newest_first` | composição: Stay = não-reparo ∧ premissa carrega para o passo de mesmo par |
| `merge_sift_step_as_is_stays_on_repair` | fato definicional do dente: as-is fica em TODO input de reparo (mesma forma de prova da divergência registrada) |
| `tagged_repair_kernel_moves_as_is_stays` | em reparo: kernel NÃO devolve Stay ∧ as-is devolve Stay |
| `tagged_stay_extends_chain` | corolário na cadeia: passo Stay estende `merge_chain` (cons do próximo passo de mesmo par) |

## Re-escopo datado (2026-09-11) — o que NÃO é provável e por quê

O wording original do P1.2 pedia "a premissa estrutural da cadeia
sobrevive ao sift_step". O núcleo disso está provado acima; a parte
"o Swap restaura newest-first" **não é provável do extract**, por duas
razões mecânicas:

1. O comparador do heap é **axioma** no extrato Aeneas
   (`Shared1A.Insts.CoreCmpPartialOrdShared0B.lt` sobre slices) — não
   há lema que conecte o resultado do comparador à ordem newest-first
   das idades (`Usize`) sem um teorema sobre o corpo do comparador,
   que não existe no extrato.
2. O `sift_step` extraído **não carrega estado de heap**: recebe três
   bools (`r_exists`, `r_lt_l`, `best_lt_hole`) e devolve um enum
   (`Stay`/`SwapLeft`/`SwapRight`) — não devolve o heap rearranjado,
   então "o par do próximo topo é newest-first pós-swap" não é uma
   frase expressível sobre a saída.

O que a ponte entrega, honestamente: a decisão analisada é sempre a
do KERNEL (não do as-is); o Stay é exatamente o não-reparo (close
registrado 0188) e preserva a premissa por congruência do par; e em
todo input de reparo o kernel move enquanto o as-is fica — o as-is
deixaria no topo o par que o kernel teria consertado.

## Verificação

- `lake build Merge`: verde (Módulo completo, incluindo os blocos
  0191/0198/0200-P1.1 anteriores).
- `grep -c sorry Merge.lean` = 0.
- Sem registro novo na escada (mesma régua dos P0.1/P1.1 do 0200:
  corolários/citações não pagam degrau).
