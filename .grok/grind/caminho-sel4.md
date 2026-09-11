# grind journal: caminho-sel4

- fire: 536
- in_progress: false
- started: 2026-09-09T03:30:32Z
- last_verdict: worked
- consecutive_noops: 0
- scheduler_id: 01a07fdf19137eb2bf4382d564c5806c
- watchdog: 60s GRIND_WATCHDOG=1
- landed: da155fab montanha-fdb-compare leftover if real_ratios.is_empty matches batch_is_empty
- next: montanha-fdb-compare leftover if !real_ratios.is_empty matches not batch_is_empty; named kernel not SA

## Fire 794 (p11) — 969640f0
RFC-0198 P1.1: Inv-WAL base + corolario de alcancabilidade. WalState.lean:
wal_state_init (log vazio, = wal_state_of 0 0 0 da producao), inv_wal_init
(base, unfold+rfl), wal_append_reach (predicado indutivo Nat-indexado de
cadeias de wal_append ok), inv_wal_reachable (corolario: passo CITA o lema
um-passo registrado wal_append_preserves_inv_wal — indutivo encadeia, nao
re-prova). Build verde primeira tentativa (1701 jobs), sorry 0. Sem mudanca
de registro (fatia de lema). COMMIT TOCAU: durante o fire a sessao paralela
fez git reset e limpou o insert NAO-commitado do teorema P0.2 em
GroupCommit.lean + os 4 arquivos de registro compartilhados ficaram com
hunks dela (floor_count do RFC-0199); meu script quebrado tinha deixado
floors 246/3 contaminando o tsv DELA — revertido pra 247/2 (estado correto
pro registro em HEAD: 2 closes) mantendo floor_count dela; gate GREEN com
script dela. P0.2 teorema PROVADO e preservado em
findings/2026-09-10-rfc0198-p02-close-registrado/theorem.snippet.lean —
landa pos-commit dela (monitor ativo). Ordem do goal virou P1.1-P1.3
primeiro (arquivos limpos); desvio registrado no plan. PROXIMO: P1.2 passo
sync/fence preserva Inv-WAL (wal_sync/wal_ack + env_crash.sync).

## Fire 795 (p12) — 4de0fa61
RFC-0198 P1.2: Inv-WAL preservacao pelo passo sync/fence + ack. WalState.lean:
wal_sync_preserves_inv_wal (Honest promove synced ate written; Lying devolve
min synced/written — fisica do crash nunca amplia barreira; via formas
fechadas privadas honest/lying + lema finish que o conjunto seguro eh
upward-closed entre as barreiras), wal_ack_preserves_inv_wal (add saturado so
avanca acked dentro de synced; ramo de estouro nao eh ok), familia indutiva
wal_write_step (append n | sync h | ack n, cada construtor carrega o ok) e
COROLARIO wal_write_step_preserves_inv_wal citando os tres lemas um-passo
(append = wal_append_preserves_inv_wal RFC-0191 P2.1, registrado; nada
re-provado). Fechada a frase "todo passo do write path que toca o WAL
preserva Inv-WAL" na escala do modelo. Armadilhas resolvidas: record
multi-linha whitespace-sensitivo; cases h : p sob if dependente (usa rw+split);
2^numBits opaco pro omega (pinar 2^64 com native_decide); simp
[saturating_add] estoura maxRecDepth (receita show Nat.min/mod). Build verde,
sorry 0 no arquivo, lean_extracts --required verde (61 libs + 5 compose),
gates depth/product/ledger GREEN (sem mudanca de registro). ProbeLoop.lean
deletado antes do commit. RFC P1.2 flipped; findings
2026-09-11-rfc0198-p12-inv-wal-passo-sync. P0.2 segue BLOQUEADO em commit da
sessao paralela (monitor 01a08e67 ativo; snippet preservado). PROXIMO: P1.3
Inv-LSM corolario indutivo k-merges em Merge.lean citando
inv_lsm_newest_first_never_non_live.

## Fire 796 (p13) — f232b71c
RFC-0198 P1.3: corolario indutivo Inv-LSM — cadeia de k merges. Merge.lean:
estrutura MergeStep (par de idades no topo do heap + kind + range_hidden),
premissas merge_step_newest_first (probe 0164 responde o mais novo no empate
— premissa estrutural POR PASSO) e merge_step_answers_live, predicado
indutivo merge_chain Nat-indexado (base = cadeia vazia, cons = topo
newest-first + cadeia k) e COROLARIO merge_chain_preserves_inv_lsm: todo
passo respondido live numa cadeia newest-first é genuinamente live (Value
nao escondido). Inducao sobre a cadeia; passo CITA o lema um-passo
REGISTRADO inv_lsm_newest_first_never_non_live (RFC-0191 P2.2) — nada
re-provado. Escolha de forma: extrato nao tem tipo de heap (sift_step é
kernel booleano de decisao); cadeia = sequencia de passos que subiram,
premissa estrutural no construtor — forma seL4 honesta na escala do modelo
(documentada no findings). Build verde primeira tentativa, sorry 0,
lean_extracts --required verde (61 libs + 5 compose), gates
depth/product/ledger GREEN (sem mudanca de registro). RFC P1.3 flipped;
findings 2026-09-11-rfc0198-p13-inv-lsm-k-merges. PROXIMO: P1.4 continua
bloqueado (concurrent.rs da sessao paralela) e P0.2 tambem; proximo
livre é P2.2 (atom df com cap 100→99 no mesmo commit) enquanto o monitor
nao reporta os compartilhados assentados.

## Fire 797 (p23) — 91789eee
RFC-0198 P2.3: herdados do 0187 seguem terminais. Fatia docs-only:
checkbox+row flipped citando o estado terminal do 0191 P2.4 (ledger
§Herdados do 0187, commitado no HEAD) — L28 user-gated, TCG/F_FULLFSYNC
nightly-experimento, N=4 aberto. Nenhum re-aberto; alargar fronteira segue
sendo movimento de ledger. Ledger NAO editado (hunk count/RFC-0199 da
sessao paralela em voo). Gates GREEN (sem mudança de registro). PROXIMO:
P2.2 atom df precisa proof_depth.tsv+residuals.json (bloqueado nos
compartilhados, igual P0.2); preparar o Lean do proximo atom df enquanto
o monitor nao dispara.

## Fire 798 (p14) — b7a9c64b
RFC-0198 P1.4: tokens write_pending_frame pagos. Board le
unpaid_script=0/17 com os DOIS tokens commitados no HEAD
(finish_group_off_lock em concurrent.rs, group_finish em db.rs) — o 2/17
do background era leitura do worktree EM VOO da sessao paralela em
2026-09-10; nenhum commit tocou os dois arquivos desde o RFC (git log
16b068c6..HEAD vazio). NENHUMA edição minha em concurrent.rs/db.rs — a
condição de fechar era o oráculo ler 0/17 e ele le. Tests nomeados do
handler 2/2 passam. MONITOR DISPAROU antes deste fire: sessao paralela
commitou 4df272d5 (rfc0199 P0.1+P0.2 — kind count no ratchet +
lsm_compact_work_bound, floor_count 0->1); compartilhados limpos vs HEAD.
PROXIMO: P0.2 (re-inserir teorema do snippet, close row, floor_close 2->3,
residuals close 3->4) e depois P2.2 (atom occ_snap + cap 100->99), um
commit por promocao.

## Fire 799 (p02) — aa5059bd
RFC-0198 P0.2: close registrado occ_batch_plan ∘ occ_conflict.
GroupCommit.lean: occ_batch_plan_member_fate_iff — iff ∀ com ∃-bind do
callee occ_conflict no corpo do loop do batch (glue por membro extraído):
TooOld vence pela flag do membro sozinha; Conflict exige flag falsa E callee
ok true em chave tocada; Ok exige flag falsa E callee ok false. Mesma regra
de computação sobre os dois corpos do molde P0.1; comportamento do loop
inteiro já pinado pelos teoremas concretos existentes. Registro NO MESMO
COMMIT: close row em close_proofs.tsv, floor_close 2->3, residuals close
3->4 (replace cirúrgico, sem sort_keys). Re-inserido VERBATIM do snippet
preservado em findings/2026-09-10-rfc0198-p02-close-registrado (prova de
2026-09-10 sobreviveu ao git reset estranho; build verde de primeira).
lean_extracts --required verde (61 libs + 6 compose — o +1 é o LsmCompactCount
da paralela); gates GREEN: depth close=3 (floor 3) residuals 4 == live 4,
product, ledger 298/265/33. ACCEPTANCE DO GOAL ATINGIDA NA DIMENSÃO P0:
ladder close registrado = 3 (floor 3). PROXIMO: P2.2 — atom
occ_snap_uses_published_ok_iff_inflight (teorema já no Flush.lean verde) +
close row atom + catalog flip (data_fate removido do par occ_snap_published)
+ floor_extract 247->246, floor_atom 31->32, cap_data_fate 100->99,
residuals idem, plant test, UM commit.

## Fire 800 (p2/P2.2) — ade73704
RFC-0198 P2.2: primeira descida de cap do RFC. occ_snap_uses_published
promovido a atom — occ_snap_uses_published_ok_iff_inflight em Flush.lean
(molde passo-30: corpo lift puro, iff = regra de computação inteira do
do-block; o if decide a base de visibilidade do snapshot OCC na janela
off-lock: seq publicada iff commit inflight). Ladder NO MESMO COMMIT: atom
row, catalog flip (data_fate removido do par occ_snap_published — sai do
pool TCB data-fate; three_teeth mantém AS-IS+twin+dst_plant exigidos),
floor_extract 247->246, floor_atom 31->32, cap_data_fate 100->99, residuals
idem cirúrgico. Planta DST 1/1 passa; lean_extracts verde (61+6); gates
depth (extract 246 floor 246, close 3, atom 32 floor 32, df 99<=99) /
product / ledger GREEN; pedra_formal 2647 ok 0 gap (161 fail = herdado).
Teorema preparado no bloqueio e aterrissado pós-commit da paralela.
ESTADO DO GOAL: P0.1✓ P0.2✓ P1.1✓ P1.2✓ P1.3✓ P1.4✓ P2.2✓ P2.3✓; resta
P2.1 (fecha com RFC-0199 P0.3 da paralela — ainda todo no RFC dela) + sweep
final (gates no HEAD + RFC sucessor no próximo id livre + status.md).

## Fire 801 (sweep) — 231addb3
RFC-0198 sweep final: 8/9 fatias done. Bloqueio externo datado em P2.1
(graduacao model->count E o P0.3 do RFC-0199 da paralela — "absorve a P2.1
do 0198" — ainda todo; no momento do sweep ela esta ATIVAMENTE nele:
ProbeLadderCount.lean nao-commitado mid-proof + edit dela no
lean_extracts.sh quebram a suite compartilhada — nao é meu, nao toquei).
Gates GREEN no HEAD: depth 246/3/32/1 df 99<=99, product, ledger 298/265/33.
Cinco modulos tocados buildam verde individualmente (1713 jobs), sorry 0;
lean_extracts GREEN no commit ade73704 (61+6, capturado no fire 800).
RFC SUCESSOR 0200 (id verificado livre, max era 0199):
alcancabilidade-completa — P0 fecha a frase seL4 completa do WAL
(wal_write_step_reach: qualquer sequencia append/sync/ack do inicial
preserva Inv-WAL, CITANDO os registrados; base inv_wal_init, passo
wal_write_step_preserves_inv_wal) + quarto close de glue; P1 base de saida
do merge + ponte sift_step<->newest-first (cita
merge_sift_step_repairs_iff); P2 cadencia. status.md: linha 0198
refreshada (8/9 + bloqueio nomeado), linha 0200 nova. MONITOR ARMADO no
registro P0.3 dela (count row scale_predict/probe_order_covering ou flip
do checkbox); quando disparar: flipar P2.1 do 0198 no mesmo commit do
re-tier registrado. Goal encerrado no estado honesto: tudo que era meu
landou; um item permanece externamente bloqueado e nomeado.

## Fire 802 (rfc0200 P0.1) — 7fe497f5
RFC-0200 P0.1: alcançabilidade da classe write-path. WalState.lean:
wal_write_step_reach (familia indutiva Nat-indexada sobre QUALQUER
construtor — append/sync/ack, qualquer ordem — a partir do log vazio) +
corolario inv_wal_write_reachable: base CITA inv_wal_init (0198 P1.1),
passo CITA wal_write_step_preserves_inv_wal (0198 P1.2). Fecha a frase
seL4 completa do WAL — o 0198 só tinha append-contas. Prova = duas
citações; build verde primeira tentativa; sorry 0; sem mudanca de
registro. Depth gate GREEN no commit (com os counts EM VOO da paralela
já contando: count=3 residual 3 == live 3 — registro dela ainda
nao-commitado, nao entro). PROXIMO: rfc0200 P1.1 merge_output_reach em
Merge.lean (base saida vazia + composicao com
merge_chain_preserves_inv_lsm), ou P1.2 ponte sift<->newest-first;
P2.1 do 0198 continua esperando o commit dela (linhas count já no
worktree).

## Fire 803 (rfc0200 P1.1) — a2e6f712
RFC-0200 P1.1: saida de merge alcancavel. Merge.lean: merge_output_reach
(familia indutiva — base empty, emit anexa AO FIM: ordem de PRODUCAO),
PONTE merge_output_reach_chain (saida lida de tras pra frente E cadeia
merge_chain — cons = emissao mais recente; List.reverse_append), corolario
merge_output_reach_preserves_inv_lsm = ponte + merge_chain_preserves_inv_lsm
(0198 P1.3) + List.mem_reverse. Zero re-prova; build verde primeira; sorry
0; sem registro. PROXIMO: rfc0200 P1.2 ponte sift<->newest-first (cita
merge_sift_step_repairs_iff).

## Fire 804 (rfc0198 P2.1 flip final) — c7b546c1
Commit da paralela assentou: 750de460 (rfc0199 P0.3 —
point_get_probes_le_levels_l0_max sobre extract ScaleKernel, linhas count
scale_predict+probe_order_covering + floor_count 1->3 no MESMO commit; o
checkbox do 0198 ela flipou no commit dela) e em seguida 67b40574 (P1.1
dela: WorkIo.lean + wal_commit_plan_at_most_one_fdatasync, count=4).
Meu flip: row P2.1 da tabela blocked->done citando 750de460, findings
sweep-final 8/9->9/9 (+ row dela na tabela de fatias), status.md linha 48
done 9/9 + escada count 4, header Status: done (convencao do 0188).
Gates no worktree limpo: depth GREEN (close 3/floor 3, atom 32/32,
count 4/4, data_fate 99), product GREEN, ledger GREEN. RFC-0198 FECHADO
9/9. Compartilhados limpos -> r200-p02 (quarto close) desbloqueado.
PROXIMO: rfc0200 P1.2 ponte sift<->newest-first (TaggedStep + cita
merge_sift_step_repairs_iff 0188 + congruencia + divergencia as-is).

## Fire 805 (rfc0200 P1.2) — 2c5a24ec
Merge.lean bloco P1.2: TaggedSift (3 bools do extract + decisao s COM
tagged_kernel_decision = decisao E do kernel), tagged_step_stays_iff_no_repair
(re-export do close registrado 0188 — rw + Result.ok.injEq),
merge_step_newest_first_congr (premissa LOCAL ao par: nao le kind/
range_hidden), tagged_stay_preserves_newest_first + tagged_stay_extends_chain
(cadeia estende por par), merge_sift_step_as_is_stays_on_repair (as_is e
CONSTANTE ok Stay no extract) + tagged_repair_kernel_moves_as_is_stays
(kernel move, as-is fica). RE-ESCOPO DATADO no RFC + findings: "swap restaura
newest-first" nao e provavel do extract (comparador axioma
CoreCmpPartialOrdShared0B.lt, sift sem estado de heap — so 3 bools).
Debug: cases t.s NAO substitui projecao dentro de hipotese (nomatch
via com unificador flex) — trace_state revelou; injEq no lugar. Build
verde 1701 jobs, sorry 0. Licoes: (1) rw em hip folded precisa unfold
antes; (2) cases em projecao de variavel nao generaliza hipoteses.
PROXIMO: rfc0200 P0.2 quarto close (floor_close 3->4) — compartilhados
limpos no momento do commit do flip P2.1.

## Fire 806 (rfc0200 P0.2) — b938e544 + 631f16b1
QUARTO close registrado: par wal_rotate_decision ∘ wal_segment_is_empty
(o passo do try_rotate_wal em db.rs — decisao, recheck inflight sob
mutex, segmento vazio pula). Teorema
try_rotate_step_rotates_iff_pins_clear_segment_live em Flush.lean:
passo dispara rotate_wal_now EXATAMENTE quando decisao = RotateWal ∧
recheck idle ∧ segmento COM dados (nunca reescreve MANIFEST ocioso).
Catalog pair NOVO wal_rotate_decision (298->299), linha close,
floor_close 3->4, residuals close 5. Build Flush verde, sorry 0.

INCIDENTE E LIXAO CRITICA DE SESSAO PARALELA: no momento do meu
commit de ledger, o indice JA continha os arquivos STAGED da paralela
(ela deu git add no meio) — commit sem pathspec engoliu 11 arquivos
dela (09c63192). Desfeito com git reset --mixed HEAD~1 (worktree
intocado — voo dela 100% restaurado, 4 untracked de volta) e
re-commitado com `git commit --only <path>` (631f16b1, 1 arquivo).
REGRA DAQUI PRA FRENTE: nunca `git commit` nu neste repo enquanto a
paralela voa — SEMPRE `git commit -- <paths>` ou `--only`; e conferir
`git diff --cached --stat` IMEDIATAMENTE antes de cada commit (o
indice e compartilhado e mutavel sob meus pes).

Desvio registrado: o ledger devia ir no MESMO commit do catalogo
(regra do ratchet); foi no commit seguinte (631f16b1) porque perdi o
arquivo na primeira passada — HEAD final consistente.

Gates no HEAD 631f16b1 (worktree temporario destacado, sem tocar o voo
dela): depth GREEN (close=4/floor 4, count=4/floor 4 no HEAD — as
linhas count 5/6 dela seguem em voo no worktree), product GREEN,
ledger GREEN (299/266/292). RFC-0200: P0.1+P0.2+P1.1+P1.2 done;
restam P2.1 (cadencia) e P2.2 (sweep).
PROXIMO: rfc0200 P2 cadencia/sweep conforme o board; ou proximo close
(quinto) se P2.1 pagar por atom.

Fire 807 (2026-09-11, rfc0200 P2.1): PRIMEIRO ATOM DATA-FATE do ciclo —
wal_sync_required a atom, commit 6cc06468. Teorema
wal_sync_required_ok_iff_client_else_db em WriteAdmission.lean: a decisao
de fdatasync por commit e single-valued (client set => client_sync, senao
db_sync); AS-IS sempre-false inalcançavel. Falhei uma vez com `simp` puro
(sobrou a = b <-> b = a); fechou com simp [eq_comm]. Escada no mesmo
commit: atom row, data_fate removido + atom_reason datado, floor_extract
246->245, floor_atom 32->33, cap 99->98. Planta DST 1/1 no kernel real,
lean_extracts ok, 3 gates GREEN. Ledger NAO muda em promocao atom (par
nao muda kind/total; forma do ade73704). --only com untracked exige git
add antes; feito em cadeia unica. PROXIMO: P2.2 sweep + flip Status done.

Fire 808 (2026-09-11, rfc0200 P2.2): SWEEP FINAL + Status done — commit
d3a1c517. Worktree destacado no HEAD 6cc06468: 3 gates GREEN (extract
245/atom 33/close 4/count 6/cap 98; ledger 299/266/33), sorry 0 em
WalState/Merge/WriteAdmission/Flush, captura em
findings/2026-09-11-rfc0200-p22-sweep-final/. Admissions recusadas
seguem always false. RFC-0200 FECHA 6/6. PROXIMO: RFC sucessora (proxima
faixa: teoremas de concorrencia, numeros do board vivo).

Fire 809 (2026-09-11, rfc0202 P0.1): rwlock_client_may_mutate a atom —
commit 7634cd77. Fila data-race do mapa de concorrencia: mutacao de Db
EXATAMENTE sob write-guard (rwlock_client_may_mutate_ok_iff_holding_write,
GroupCommit.lean); AS-IS mutar-apos-soltar inalcançavel. Build verde
primeira tentativa (unfold + simp [eq_comm]). Escada no mesmo commit:
cap 98->97, floor_atom 34, floor_extract 244. Planta DST 1/1, gates 3x
GREEN, extracts ok. Descoberta pro P1.2: header do GroupCommit.lean
registra group_validate como partial_fixpoint (irredutivel a defeq) —
queda honesta do candidato ao close ja tem base in-tree. PROXIMO: P0.2
occ_member_fate a atom.

Fire 810 (2026-09-11, rfc0202 P0.2): occ_member_fate a atom — commit
85309685. Fila lost-update: destino do membro OCC EXATAMENTE TooOld >
Conflict > Ok (occ_member_fate_ok_iff_precedence, GroupCommit.lean);
AS-IS nunca-abortar inalcançavel; caller link validate_occ_batch.
Build verde primeira tentativa (unfold + cases duplo + simp [eq_comm]).
Escada no mesmo commit: cap 97->96, floor_atom 35, floor_extract 243.
Planta DST 1/1, gates 3x GREEN, extracts ok. P0 do 0202 FECHADO (2/2).
PROXIMO: P1.1 fila deadlock (ponte wait_for_deadlock ou recusa datada).

Fire 811 (2026-09-11, rfc0202 P1.1): ponte wait_for_deadlock — commit
6bab9882. Fila deadlock paga: as 3 arestas de saida de um passo do
detector 2PL com hipoteses de lookup (Locktab.lean); iff completo do
ciclo fica TCB (HashMap axioma) — fronteira datada no EXTRACT.md.
Duas lições: (1) rw sozinho nao reduz binds do do-block — simp fecha,
simp [hkey] expoe lookup pos-normalizacao; (2) gate do registro exige
forall LITERAL no texto do enunciado — binders diretos nao passam
(reescrevi os 3 em forma forall->). Escada no mesmo commit: cap 96->95,
floor_atom 36, floor_extract 242. Planta DST 1/1 (rocksdb-compat),
gates 3x GREEN. PROXIMO: P1.2 quinto close — group_validate e
partial_fixpoint (documentado no header do GroupCommit.lean), queda
honesta para o proximo par com bind de callee tratavel.

Fire 812 (2026-09-11, rfc0202 P1.2): quinto close — commit 5f676964.
group_validate (candidato do RFC) e partial_fixpoint: queda medida,
caiu para o par do board `bearer` (bearer_token_from_value,
auth_kernel.rs, handler authorize). Pago: fate iff 7-vias do output
inteiro sobre a cadeia trim/is_empty/split_once_ws + gates de scheme
(Auth.lean, bearer_token_from_value_fate_iff); core.str segue axioma
(quantificado existencialmente no iff). Escada no mesmo commit:
floor_close 4->5, residuals close 5->6 (5 registrados + 1 twin
sem extracao); cap/floor_extract intactos (par sem data_fate; par de
close nao pede cirurgia de catalogo). Gates 3x GREEN, extracts ok
(61+12), planta DST pedradb-http 1/1, Auth build verde sorry 0.
Licoes: ok some (x, y) nao elabora — ok (some (x, y)); split de
Bool-if entrega negacao no caso falso — simp only [Bool.not_eq_true]
at h antes do rw; rintro monolitico aninhado nao parseia — rinto
simples + rcases em estagios. PROXIMO: P2.1+P2.2 — recusa do
escalonador no RFC + sweep em worktree destacado + flip 0202 Status
done; depois RFC sucessora (numero livre, ela ja tomou 0203 draft).

Fire 813 (2026-09-11, rfc0202 P2.1+P2.2): fechamento 6/6 — commit
bacd770c. Recusa do escalonador REGISTRADA no RFC (nenhum flip):
lock_interleavings_admitted segue ok false, planta
claim_lock_interleavings_refused_after_put 1/1 no HEAD; cadencia atoms
paga no RFC = 3 promocoes (cap 98->95). Sweep em worktree destacado do
bacd770c: depth/product/ledger GREEN (close=5 floor 5, residuals
6/36 == live, extract 242, data_fate 95), sorry 0 nos 3 wrappers
tocados, 3 admissions recusadas confirmadas no HEAD. RFC-0202 Status:
done. Escada final da ronda: extract 242 / close 5 / atom 36 / count 7
/ cap 95. PROXIMO: RFC sucessora (proximo numero livre — ela tomou
0203 draft; conferir ls docs/rfc) com numeros vivos do board; depois
auditoria final com capturas.

Fire 814 (2026-09-11, rfc0202 fechamento + rfc0204 sucessora): ronda 2
completa. RFC-0202 6/6 em 5 commits de promocao (7634cd77, 85309685,
6bab9882, 5f676964, bacd770c) + RFC-0204 sucessora (b415382b, proximo
numero livre — 0203 era dela). Tese 0204 ancorada no board vivo:
composicao off-lock so como dentes concretos (group_publish sem
registro), 91 pares data_fate com extracao sem teorema (cluster
store/recovery), db_rs_extracted False / handler_loc 112092 sem
fronteira datada. Auditoria final no HEAD b415382b (depois de 4 lands
dela no meio): worktree destacado gates 3x GREEN, 3 admissions
recusadas, 6 commits meus sem path dela/journal, capturas no scratch
(gates-sweep-final, promo-commits, rfc0202-final, board-after, plants,
tests). Escada final: extract 242 / close 5 (residual 6) / atom 36 /
count 7 / cap 95. PROXIMO (ronda 3): implementar 0204 — P0.1 close
group_publish (may_publish_group iff forall, floor_close 5->6), P0.2
atom vote_decision (cap 95->94).
Fire 814-correcao: sucessora renumerada 0204->0205 (commits 0d0b4bdd +
8b2ffacf) — a sessao paralela aterrissou o propio 0204 (5865aaf4)
depois do meu b415382b; arquivo dela intocado, meu renumerado.

Fire 815 (2026-09-11, rfc0205 P0.1): sexto close — commit a seguir.
group_publish pago: may_publish_group_ok_iff_wal_io_ok (GroupCommit.
lean) — corpo e lift puro ok wal_io_ok, molde honesto =
dir_sync_required (pure-lift), NAO o molde bearer (corpo sem bind nao
tem cadeia de callees — diferenca documentada no findings). Escada no
mesmo commit: floor_close 5->6, residuals close 6->7. Gates 3x GREEN,
extracts ok (61+12), planta pedradb-core 1/1, build verde sorry 0.
PROXIMO: P0.2 atom vote_decision (catalog:vote, cirurgia data_fate,
cap 95->94, floor_atom 36->37, floor_extract 242->241).
