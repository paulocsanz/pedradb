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

Fire 816 (2026-09-11, rfc0205 P0.2): primeiro atom do cluster
store/raft — vote_decision_fate_iff (Vote.lean): fate forall dos DOIS
construtores (WouldGrant iff mesmo-termo E can_vote E log_up_to_date;
Deny c.c.), totality derivada do spec-match, grant-side reusado do
P40 (vote_decision_iff). Escada: cap 95->94, floor_atom 36->37,
floor_extract 242->241. Planta DST pedradb-store 1/1
(vote_decision_on_live_queued_is_not_ok), gates 3x GREEN no HEAD,
extracts ok (61+13). ATENCAO povo do index compartilhado: a linha do
registro caiu no commit da sessao paralela 392280e3 e o resto do
pacote no snapshot dela 07b6acad (que tambem commitou este journal —
violacao dela, nao minha); meu commit proprio foi so o reparo de
build c2d1ab65 (10 linhas mortas de OpenOptions no pedradb-store,
campos removidos pelo snapshot a5ccc131 dela). PROXIMO: P1.1
composicao forall off-lock em ComposeConcurrent.lean (sem registro;
compor com o close registrado do Flush, nao duplicar).

Fire 817 (2026-09-11, rfc0205 P1.1): composicao forall off-lock
commitada 9962a302. Pago: (a) concurrent_publish_fate_forall; (b)
wal_rotate_decision_fate_forall + as-is (disjuncao EXATA do corpo:
RotateWal iff registro limpo nos 5 campos; mutante droppa pin_live);
ponte try_rotate_step_rotates_iff_all_clear_record COMPNDO com o close
registrado do Flush (registro->passo), sem duplicar; 4 dentes ->
corolarios por instanciacao. Licao Lean nova: cases sobre campo de
registro so reescreve o goal SE o def ja estiver desdobrado (simp
only [def] ANTES dos cases — senao o campo nao ocorre sintaticamente e
os cases nao substituem nada; sintoma: sobras com cadeias de
implicacoes). rw de padrao grande com bind/match falha casamento —
usar Iff.trans com args explicitos e rw so da equacao pequena.
SEM registro (compose atravessa 2 kernels; registro exige par unico) —
motivo em findings. Twins 1/1, gates 3x GREEN, escada intocada.
PROXIMO: P1.2 primeira promocao recover_must_apply (pure-lift Bool,
molde dir_sync_required).

Fire 818 (2026-09-11, rfc0205 P1.2): cadencia data-fate 2/2 FECHADA.
1/2 e356a071 atom catalog:recover_apply (recover_must_apply_fate_iff:
re-apply iff commit>applied; as-is pula tudo), 2/2 34dbb666 atom
catalog:recover_drop_orphan (recover_drop_orphan_seg_fate_iff: drop
iff seg_index>new_hi; as-is preserva orfaos). Escada no alvo: cap
94->92, floor_atom 37->39, floor_extract 241->239, residuals
7/39, data_fate 92. Licao Lean: '>' de U64 no extrato Aeneas e
Prop-valued (corpo = ok (decide (a > b))) — enunciar RHS como Prop
pura e casoar o decide com of_decide_eq_true/false; (a > b) = true
no enunciado QUEBRA o parser/elaborador. proof_depth.tsv e
espacado (floor_atom 37), NAO tabulado. Plantas DST 1/1 cada
(three_teeth_queued). Gates 3x GREEN apos cada commit. PROXIMO:
P2.1 fronteira datada do handler em EXTRACT.md (TCB nomeado, 3
admissions recusadas).

Fire 819 (2026-09-11, rfc0205 P2.1+P2.2): RFC-0205 DONE 6/6 no commit
79ea30d5. P2.1 = 93c9434a (secao datada EXTRACT.md: proof-term cobre
239/6+39+7/18 compose; TCB 4 categorias; 3 admissions reiteradas).
P2.2 = worktree destacado em 93c9434a: 3 gates GREEN, extracts ok
61+18, sorry 0 nos 4 wrappers, admissions always false; captura em
findings/2026-09-11-rfc0205-p22-sweep/. Licao de metodo: worktree de
sweep precisa ficar DENTRO de software/ (lean_extracts.sh resolve o
backend aeneas por caminho RELATIVO; de /var/folders nao resolve).
PROXIMO: RFC sucessora 0206 — fechar o seam store/raft: 66 dos 92
data_fate pendentes sao do cluster (31 l28.rs, 26 membership_kernel,
6 txn_kernel, + commit_raft/grant_persist/compact_unleft).

Fire 820 (2026-09-11, rfc0205 auditoria final + sucessora): ROUND 3
FECHADO. RFC-0205 done 6/6 (tabela 6/6 apos flip cirurgico 159bb3f9
pego na auditoria). Auditoria: promocoes d4ea73f4/e356a071/34dbb666
cada uma com teorema+registro+floors no mesmo commit (verificado por
CONTEUDO do git show, nao stat truncado); meus 8 commits sem
journal/paths dela; plantas de recusa 3/3 verdes no HEAD 4a598c3e
(lock_interleavings, media_durable, forall_schedules@depth2);
capturas no scratch (promo-commits/gates-sweep/plants/rfc0205-final/
board-after). Sucessora: DUPLA colisao de numero (minha 0206
e88377b5; dela 0206->286fb7bb->renumerou a dela para 0207 3ad7f06e;
minha renumerada 0206->0208 4a598c3e, conteudo intocado). RFC-0208
"fechar o seam store/raft": 66 dos 92 data_fate pendentes sao do
cluster (31 l28, 26 membership, 6 txn, grant_persist, commit_raft,
compact_unleft); P0 fecha o trio raft, P1 composicao do cluster +
cadencia membership x4, P2 abre o bloco l28 x2 + plano datado; alvo
cap 92->84, floor_atom 39->47, floor_extract 239->231. Licoes do
round: cases sobre campo so reescreve com def desdobrado ANTES;
Iff.trans explicito no lugar de rw de padrao bind/match; '>' U64 e
decide de Prop; proof_depth.tsv e espacado; worktree de sweep dentro
de software/ (aeneas relativo); colisao de RFC: renumerar A MINHA
cirurgicamente, dela intocada.

Fire 821 (2026-09-11, rfc0208 P0): seam raft 1/2+2/2 FECHADO.
7adc0cf1 atom grant_persist (grant_after_persist_fate_iff: grant iff
WouldGrant e persist Ok) e ef36c681 atom commit_raft
(propose_ack_ok_fate_iff: ack iff index<=commit_index). Escada:
cap 92->90, floor_atom 39->41, floor_extract 239->237. Claim datada
MEDIDA: vote_kernel.rs e commit_kernel.rs zero data_fate pendente
(membership 26 nomeados para cadencia; o "tres kernels" do texto
original era overclaim — flipado honesto). Plantas 1/1, gates 3x
GREEN apos cada. PROXIMO: P1.1 composicao do cluster em
ComposeStoreRaft.lean (cadeia eleicao vote->grant via bind + trio
recovery sobre atoms registrados; vote_decision_total e private —
precisa virar publico).

## Fire 822 — rfc0208 P1.1: composição do cluster (faec9c9d)
- ComposeStoreRaft.lean (19º compose lib, lakefile+lean_extracts.sh no mesmo commit):
  election_grant_chain_fate (bind de vote_decision × grant_after_persist sobre os
  dois atoms registrados; Deny/persist-Err via vote_decision_total, tornado público)
  + recovery_fate_composed (triple recover_apply × recover_drop_orphan × node_counts).
- Zero sorry; twins DST do cluster 7/7 (three_teeth_queued); gates 3× GREEN (sem
  promoção — composição não é par do catálogo, motivo do não-registro em findings).
- 3 erros de tactic na primeira compilação (rfl onde precisava da hipótese hd;
  rw [hd] antes do simp numa equação entre construtores; aridade dos conjuntos do
  Deny — 3, sem persist). Molde do bind-composição bankado: bind_ok_inv/bind_intro
  + rcases dos dois iff registrados.

## Fire 823 — rfc0208 P1.2: cadência membership ×4 (c3bfeb72, 9188ec08, 08016c7d, 8837e2ad)
- 4 atoms, 4 commits, números EXATOS do RFC: cap 90→86, floor_atom 41→45,
  floor_extract 237→233. removed_steps_down e disk_membership_overrides_cli
  (pure-lifts), high_water_at_least (trait-default Ord::max; semântica do lt
  escalar = ok (decide (x<y)) por rfl), joint_still_active (PRIMEIRO USO no repo
  da ponte de specs da Aeneas: eq_homo_spec + spec_imp_exists — corpo que atravessa
  trait deixa de ser opaco).
- Lições bankadas: do-block do backend reduz com simp COMPLETO (o bind é da instância
  Monad — bind_ok direto não casa); iff residual de simetria fecha com `exact eq_comm`;
  extração de Prop de hbeq (spec) fecha com hbeq.mp rfl / fun h => absurd (hbeq.mpr h).

## Fire 824 — rfc0208 P2.1: banda l28 ×2 + plano datado do bloco (b5e15cb9, 98d7b960)
- 2 atoms, 2 commits, números EXATOS do RFC: cap 86→84, floor_atom 45→47,
  floor_extract 233→231. l28_tcp_left (reporte de saída ⟺ disco) e l28_tcp_hw
  (high-water move ⟺ inventário commitado mantido) — pure-lifts ok b em L28.lean,
  molde cases b <;> cases v <;> simp (4ª e 5ª aplicação).
- Plantas TCP REAIS verdes (protocolo TCP entre nós, ~235s cada): remove_member_
  left_on_disk e high_water_after_remove (tests/l28_real_tcp.rs).
- Plano datado dos 29 data_fate restantes em EXTRACT.md: 22 pure-lifts extraíveis
  (identidade literal num Bool; risco de corpo opaco ZERO) em cadências de 4, cada
  um com planta real nomeada — meta relativa: cap 84→62, floor_atom 47→69,
  floor_extract 231→209; + 7 fantasmas de catálogo (add/cnew/svget/newget/jleft/
  caught/grown nomeiam fn que NÃO existe — kernels TCP terminam em pj) = conserto
  de catálogo ou aposentadoria, NUNCA gate de identidade inventado.
- Sessão paralela ativa em pedradb-core (wal/*, wal_buffer_kernel.rs) — commits
  sempre --only com meus caminhos; commit dela (6bb34dfc) caiu entre os meus dois.

## Fire 825 — rfc0208 P2.2 + flip: sweep em worktree destacado, 0208 fechado 6/6 (9a20cb25, 637eda76)
- Worktree destacado DENTRO de software/ (pedradb-wt0208) no HEAD 98d7b960: gates 3×
  GREEN, extracts --required ok (61 libs + 19 compose; 1929 jobs lake), sorry 0 nos 5
  wrappers tocados, admissions always false (recusas plantadas). Worktree removido com
  --force (só pycache sujo).
- Correção datada da aritmética do slice: restante vivo do cluster = 58 nomeados
  (22 membership + 29 l28 + 6 txn + 1 compact_unleft), NÃO 55 — o texto subtraía o
  trio do 0205 que já estava fora dos 66. Pool vivo 84 = cap ✓.
- Escada final do 0208 contra o início: extract 239→231, atom 39→47, cap 92→84 —
  alvos exatos do RFC; 8 atoms (um commit cada) + 1 compose lib.
- SESSÃO PARALELA abriu a 0209 (c064d621, WAL staging) — sucessora precisa do
  próximo número LIVRE (checar ls docs/rfc/ na hora; precedente da dupla colisão).

## Fire 826 — rfc0210: sucessora do 0208 autorada (80c6acde)
- Número: 0210 (0209 tomada pela sessão paralela em c064d621 — WAL user-space staging).
- Tese com números vivos: pool 84, L28 = 29 (22 pure-lifts risco-ZERO pelo plano
  datado + 7 fantasmas). P0 = cadências 1/4+2/4; P1 = 3/4+4/4 e o VEREDITO dos
  fantasmas (conserto se caminho vivo, senão aposentadoria datada sem quebrar
  espelhos/âncoras 0203/0204 — conta ancorada quebrada ⇒ recusa registrada, nunca
  força); P2 = cadência final (bloco ZERO data_fate) + composição ∀ da remoção TCP
  em ComposeL28.lean + sweep. Alvos: cap 84→62, floor_atom 47→69, extract 231→209.
- Out-of-scope com os bans: admissions, seL4, db.rs inteiro, HashMap, ∀π, escada
  dela, add-member joint como produto (catálogo não dirige o produto).

## Fire 827 — auditoria final round 4: limpa
- 8 commits de promoção, cada um com EXATAMENTE +1 linha de registro (git show por
  conteúdo); 4 não-promoções (compose/sweep/flip/RFC) sem registro espúrio.
- 0 paths proibidos nos 12 commits do round (journal, findings 0192-0196, kernels
  e âncoras dela — nada). Commits dela (6bb34dfc, c064d621) intactos.
- 3 admissions recusadas VERDES no HEAD (media_durable, forall_schedules,
  lock_interleavings — 3 passed). Capturas no scratch novo
  (implementer/auditoria-round4-rfc0208.md).
- ROUND 4 FECHADO: 0208 done 6/6 nos números exatos (extract 239→231, atom 39→47,
  cap 92→84); sucessora 0210 aberta. PRÓXIMO ROUND: implementar 0210 P0.1
  (cadência l28 1/4: dterm, part, apply, napply — 4 commits, plantas TCP reais).

## Fire 828 — rfc0210 P0+P1.1: 12 atoms do bloco l28 (9a0869ee…este)
- P0 (8 atoms): cadências 1/4+2/4 — dterm, part, apply, napply, trunc, odrop,
  abort, nowms. P1.1 (8 atoms): cadências 3/4+4/4 — hist, fence, clear, pre,
  peer, lid, rdr, dsc. Números EXATOS em cada fechamento: cap 84→76 (P0),
  76→68 (P1.1); floor_atom 47→55→63; floor_extract 231→223→215.
- Molde pure-lift cases b <;> cases v <;> simp: 13ª+ aplicação — ZERO erros de
  prova em todo o round 5 até aqui; script de cirurgia reutilizável no scratch
  (promote_l28.py) derrubou cada promoção para ~1 min.
- Plantas TCP REAIS em paralelo (4+8 simultâneas): 16/16 verdes, 234–391s cada.
  Lição: lançar as plantas do lote ANTES das promoções e colher os resultados
  antes/durante os commits finais do lote.
- PRÓXIMO: P1.2 veredito dos 7 fantasmas (medição add-member → recusa datada
  SEM aposentadoria se quebrar âncoras dela), depois P2.1 cadência final ×6.

## Fire 829 — rfc0210 P1.2+P2.1+P2.2+sucessora: round 5 FECHADO (fe236f91…4cba62a2)
- Correção do Fire 828: P0+P1.1 = 16 atoms (não 12) — o corpo estava certo.
- P1.2 (fe236f91): 7 fantasmas APOSENTADOS com recusa datada — medição a
  fresco: cluster_real.rs sem add-member, handlers/planta inexistentes, zero
  defs Lean; catálogo 299→292, data_fate 68→61, cap 61, single_artifact 285
  (corrigiu stale 291 vs live 292), marker ledger, nota R-joint. Gates +
  host_anchor_table + test_proof_vs_campaign + test_twin_mutation verdes
  ANTES e DEPOIS. Lição: pedra_formal compara glue.sa/data_fate com o vivo —
  atualizar junto; o marker do ledger carrega total/campaign/sa/aeneas.
- P2.1 (fafbbfa4…e627702c, 6 commits): pld, std, hnt, slot, sth, pj — cap
  61→55, floor_atom 68→69, floor_extract 210→209. Plantas 6/6 verdes
  (pld 231.8s, std 369.0s, hnt 366.6s, slot 349.2s, sth 22.2s, pj 1.8s —
  sth/pj são ctor-sobre-plantado, não pagam kill-wait). Medido: 26 pares
  l28, ZERO data_fate.
- P2.2 (80f2344f + 48db12b4): ComposeL28.lean (20ª compose lib) —
  protocolo de remoção composto (left ∧ hw) + fused (bind). Lição: o gate
  extracts recusa a PALAVRA sorry até em comentário ("Zero sorry." no
  header falhou; reescrito "No holes"). Lição 2: Aeneas bind não reduz por
  simp sozinho — `simp [Aeneas.Std.bind]` após cases fecha.
- Sweep no worktree destacado dentro de software/ (mold 0202): gates no
  worktree, extracts/sorry na árvore principal. RFC-0210 Status done.
- Sucessora 4cba62a2: RFC-0211 — drenar o bloco cluster (membership ×22 +
  txn ×6 + compact ×1 = os 29 nomeados; cap 55→26, atom 69→98, extract
  209→180; molde 0208 P1.2 de corpo REAL, não pure-lift).
- PRÓXIMO ROUND: implementar 0211 P0.1 (cadência membership discard ×4:
  discard_uncommitted, discard_leader, drop_preimages, force_clear).
- Correção do Fire 829: a sucessora virou RFC-0212 (5db39a2c) — a 0211 é dela
  (escalonamento rmw mc4, a262f095); colisão ⇒ renumerar a minha.

## Fire 830 — rfc0212 P2.1+P2.2+sucessora: round 6 FECHADO (0e86d730…9df25b47)
- P2.1 ×7 (0e86d730, 6b8d3454, de09a2c3, 11fc86f1, a795d062, 4185c2fc,
  11354e27): discard_cut, leftover_txn_is_aborted, next_txn_id_after,
  prepare_error_aborts_earlier, reserve_si_gen, unreserve_si_gen (StoreTxn)
  + compact_through_unleft (StoreCompact). Números EXATOS: cap 33→26,
  floor_atom 91→98, floor_extract 187→180. BLOCO CLUSTER 29/29 (membership
  22 + txn 6 + compact 1) ZERO data_fate medido ao vivo. 1 promoção = 1
  commit ×7 auditado (grep -c "^+theorem" = 1 em cada).
- Lição P2.1: eu tinha appending os 6 teoremas de uma vez e o commit --only
  leva o working tree INTEIRO do arquivo — o primeiro commit carregou os 6.
  Reset soft + truncar o wrapper para 1 teorema + reapendar um a um por
  commit (bloco salvo em {SCRATCH}/p21_pending_theorems.lean). Sempre
  1 teorema por commit desde o primeiro.
- Lição P2.1 2: StoreTxn NÃO estava no LIBS do lean_extracts.sh — 23
  teoremas sem gate desde o RFC-0191 (nada importava o wrapper). Inscrito
  no primeiro commit do P2.1 (62 libs).
- Plantas: 4 novas em three_teeth_queued.rs via LiveQueued; técnica
  broadcast_append (log RAM do líder): prepare persiste no índice de commit
  (discard_cut), Puts carimbam si_gen 2,3 distintas (reserve/unreserve);
  compact via unleft_applied_joint_index==None. Regressão módulo 85→86
  passed (agora 150 com twin P2.2).
- P2.2 (9f94b167 + c892b57d): ComposeStoreFinish.lean (21ª compose lib) —
  queued_finish_chain_fate sobre os 4 atoms do fim-de-fila (discard_leader ×
  discard_uncommitted × persist_fence × persist_hist), v = localidade para
  todo in_ids (0136 sobrevive); SEM registro TSV (razão em findings). Twin
  kernel queued_finish_from_counts + planta viva: put não cometido na réplica
  4 (prev DELA, técnica da planta discard) → leave remove 4 → fence
  persiste "abort" na removida → discard derruba o sufixo (sent_through
  neutralizado). Hook de drift Aeneas: mudou membership_kernel ⇒ re-extract
  e re-carimbar SOURCE.store_membership no commit.
- Sweep: worktree /Users/paulo/software/pedradb-wt-r0212 no HEAD 9f94b167 —
  gates 3× GREEN + test_proof_vs_campaign ok; árvore principal extracts
  "62 libs + 21 compose", sorry 0. RFC-0212 Status done.
- Auditoria final: 1 teorema/commit ×7; nenhum path dela; admissions
  3/3 verdes no HEAD (claim_media_durable_refused_after_fsync_ok,
  forall_schedules_admitted_on_live_group_is_not_ok,
  claim_lock_interleavings_refused_after_put); gates GREEN no HEAD
  (292/266/26, data_fate 26<=26). Capturas em {SCRATCH}/audit_*.
- Sucessora 9df25b47: RFC-0213 — drenar o storage (wa 9 + lookup 4 + flush
  3 + cf 2 + leveling 2 + 6 singletons = 26; cap 26→0, atom 98→124, extract
  180→154; CATÁLOGO ZERO data_fate). WalRecover + Reopen fora do LIBS
  (buraco classe StoreTxn) — inscrição na P2.1 dela.
- PRÓXIMO ROUND: implementar 0213 P0.1 (cadência write_admission ×9).
- Fronteira respeitada: pedradb-core só leitura (admissions); commit dela
  741aecbf entrou no meio do meu fecho sem conflito.

## 2026-09-12 — round 7 FECHADO: RFC-0213 done, CATÁLOGO ZERO data_fate (26 promoções + compose + sweep)

- P0.1 wa×9 (2979b00c…0b88780c FECHAMENTO) + P0.2 lookup×4 (9b780470…d2749e24)
  + P1.1 flush×3+cf×2 (394a08d0…f604d0ee) + P1.2 leveling×2+singletons×4
  (ced458f1…44bd1b67 FECHAMENTO; 8898bb93 cap-only visible_at) + P1.3 veredito
  (40afb3b0: 0 ausentes nas 24) + P2.1 wal finais ×2 (6a538512 wal_recover atom
  FRESH; 57c1277e dictionary_link CAP-ONLY — atom desde 2026-09-10, fate-iff
  fortalecido, pagou só o cap) — CATÁLOGO ZERO data_fate (292 pares medido ao
  vivo). Escada final 0213: cap 26→0, floor_atom 98→122, floor_extract 180→156.
  WalRecover + Reopen INSCRITOS no LIBS nesta data (62→64; extratos
  re-carimbados byte-idênticos, diff vazio).
- P2.2 (3782ced7 + 7f1c7cf4): ComposeStorageWrite.lean (22ª compose lib) —
  storage_write_path_recovered_iff: admission portão → plano cerca → recovery
  corta sobre os atoms write_admit × wal_commit_plan × torn_tail_needs_cut,
  v true EXATAMENTE sob a conjunção quádrupla para TODO input; SEM registro
  TSV (três kernels — razão datada em findings p22). Twin kernel
  storage_write_recovered(+as_is) + planta
  storage_write_recovered_on_live_stall_fence_torn_is_not_ok (4 quadrantes +
  as-is mente nos três eixos); extrato re-carimbado
  (aeneas_write_admission.sh, +49 linhas). Hook drift satisfeito.
- Sweep: worktree /Users/paulo/software/pedradb-wt-r0213 @ 3782ced7 — gates
  3× GREEN + test_proof_vs_campaign ok + extracts "ok lean extracts (64 libs
  + 22 compose)" build-completo do zero (1944 jobs), sorry 0; worktree
  removido. Nota datada EXTRACT.md: bloco storage DRENADO — catálogo inteiro
  292 pares ZERO data_fate; restam nomeados: ZERO (0211/0212/0213 drenados).
  RFC-0213 Status done.
- Auditoria final: 1 teorema/commit em TODAS as 26 promoções (sweep/veredito
  = 0, compose = 1); nenhum arquivo dela commitado por mim (concurrent.rs só
  no commit DELA a5e18c75, sessão 0211); admissions 3/3 verdes no HEAD
  (claim_media_durable_refused_after_fsync_ok,
  forall_schedules_admitted_on_live_group_is_not_ok,
  claim_lock_interleavings_refused_after_put); depth-floor GREEN no HEAD
  (extract=156/atom=122/data_fate=0<=0). Capturas em {SCRATCH}/r0213_audit_*
  + r0213_p22_*.
- Sucessora 232c5cdd: RFC-0214 — drenar a espinha de durabilidade ao degrau
  átomo (wal_state ×6 + env_crash ×6 + cqe ×5 + write_ack ×3 = 20 pares;
  wrappers já no LIBS, zero buracos): floor_atom 122→142, floor_extract
  156→136, cap_data_fate 0<=0 imutável; P1.2 veredito datado para classe
  campanha; P2.1 composição ∀ env→wal→ack; P2.2 sweep + EXTRACT.md + done.
  Fora de escopo nomeado: group_commit ×11 (escalonador permanece ok false).
- PRÓXIMO ROUND: implementar 0214 P0.1 (wal_state ×6 ao átomo).
- Fronteira respeitada: 4 arquivos sujos dela intactos (concurrent.rs,
  caminho-sel4.md, serial-fillin10.prevstate, rfc0029 stdout.json, __pycache__);
  commit dela a5e18c75 entrou no meio do round sem conflito.

## 2026-09-12 — round 8 FECHADO: RFC-0214 done, espinha de durabilidade no degrau átomo (20 promoções + compose ∀ + sweep)

- 20 promoções 1 teorema/commit, gate GREEN em todas: wal_state ×6
  (d6179165, bb0f8e08, eb5c65b4, 8f370758, 2a23f62a, ca40cfde), env_crash
  ×6 (2977e49e, 44bed493, 49f8461e, 02dc3a64, 3bacca3d, 1130ec51), cqe ×5
  (b5350a8c, 003fa79b, 0db60735, 61d6946b, c57a78d1), write_ack ×3
  (0d7324da, 4893f009, 42891839) — floor_atom 122→142, floor_extract
  156→136, close=6/count=7/data_fate 0<=0 imutáveis.
- Correção LIVE no meio do round (0d7324da): puts sync single-client
  tomavam submit_one → lone_commit e BYPASSAVAM o ledger do WriteAck;
  fix liga o ledger ao caminho pinned (on_append/on_barrier/on_ack +
  assert_inv em lone_commit quando verified) — planta DST on live
  (2 puts, crash+reopen, ambos gets sobrevivem) verde antes do commit.
- P1.2 veredito datado (8b5d5dd4): nenhum medido recusado, 20/20 — sem
  gate inventado.
- P2.1 composição ∀ (8e2bb091): ComposeDurabilitySpine.lean — spine_step
  (append/barrier/ack com futuros ok), spine_reach, pontes ok DERIVADAS
  das três iff (corpos extraídos nunca reabertos para futuros ok),
  spine_inv_every_reach (Inv-WAL acked⊆synced⊆written de TODO caminho,
  indução em spine_reach), coroa spine_d1_every_reach (D1 do prefixo
  acked sobre TODO corte tornado — abre inv_wal+d1_modelo(+crash_legal)
  UMA vez, montagem); twin durability_spine_kernel.rs (SpineStep,
  spine_replay com assert_inv por passo, spine_replay_as_is, 2 tests) +
  planta DST on live no perfil verificado pinned; inscrição lakefile +
  COMPOSE; SEM TSV com razão datada em findings (atravessa 3 átomos,
  não um par único). Moldes Lean aprendidos: literal de estrutura em
  construtor de indutivo tem que ser 1 linha; `cases hs with` unifica o
  `l` do construtor com o `l` em escopo (nomear só binders novos);
  `show` falha defeq em projeção de let __src → `dsimp only` antes;
  ifs rebaixados a `= true` sob binds precisam `liftFun2` no simp set
  para beta a continuação.
- P2.2 sweep (85855e56): worktree /Users/paulo/software/pedradb-wt-r0214
  @ 8e2bb091 DENTRO de software/ — depth-floor GREEN (extract=136 floor,
  atom=142, close=6, count=7, data_fate=0), inventory 7/7 terminal,
  twins 7/7 bound (TSV máquina), test_proof_vs_campaign ok (3 ok),
  extracts 64 libs + 23 compose build do zero 1946 jobs ok, sorry 0 nos
  cinco wrappers da rodada; nota datada EXTRACT.md (linha 600: ladder
  122→142/156→136, "Restam no degrau extrato: 136"); RFC-0214 Status
  done; worktree removido (--force só pyc regenerado).
- Auditoria final round 8 (HEAD 85855e56): 1 teorema/commit em TODAS as
  20 promoções (compose P2.1 = 5 teoremas, veredito/sweep = 0);
  NENHUM arquivo dela no range 7f1c7cf4..85855e56; admissions 5/5
  verdes no HEAD (claim_media_durable_refused_after_fsync_ok core+world,
  forall_schedules_admitted_on_live_group_is_not_ok,
  claim_lock_interleavings_refused_after_put,
  cqe_ring_model_is_not_admitted io-uring); depth-floor GREEN no HEAD;
  extracts --required exit 0 no HEAD (1939 jobs). Capturas
  {SCRATCH}/r0214_audit_{gate,admissions,extracts}.txt +
  r0214_sweep_* + r0214_p*_*. Baseline vermelho pré-existente
  documentado (pedradb-core 23, pedradb-sim 2 — medido no HEAD limpo
  c57a78d1 ANTES do round; não é do 0214).
- Sucessora faeb1f98: RFC-0215 — coroa de produto no degrau átomo.
  Medição ao vivo (candidates.py @ 85855e56): TODOS os boards de
  máquina zerados (script 0/17, compose 0/17, concorrência 0/5, scale
  0/3, produto 0, A/B/D/C vazios, data_fate=0, atom_to_close none);
  cartoon_twin=4 TODO em montanha-fdb-recipes (skip, regra 13). Restam
  136 no degrau extrato; o bloco de maior valor seL4 é a COROA DE
  PRODUTO: spec ×4 (Properties.lean: d1_holds, r1_answer_ok, t1_holds,
  c1_holds — hoje só placeholder `: True`), modelo ×4 (d1_modelo,
  r1_modelo, t1_modelo, c1_modelo), fate ×2 (d1_put_ok,
  c1_advance_commit); P1.2 ComposeProductCrown (11ª compose): espinha →
  d1_modelo ÁTOMO (corpo não reaberto) → d1_holds spec — a garantia de
  produto D1 como teorema composto sem buraco de extrato no meio;
  P2.1 http ×6 (Auth/Form já no LIBS). Alvo: floor_atom 142→158,
  floor_extract 136→120.
- PRÓXIMO ROUND: implementar 0215 P0.1 (spec ×4 ao átomo em
  Properties.lean).
- Fronteira respeitada: arquivos sujos dela intactos
  (caminho-sel4.md, serial-fillin10.log.prevstate, rfc0029 stdout.json,
  __pycache__); nenhum stash tocado; concurrent.rs só mexido no commit
  do fix lone_commit (0d7324da) com planta verde antes.
## 2026-09-12 — round 9 FECHADO: RFC-0215 done, coroa de produto no degrau átomo (16 promoções + compose espinha→coroa + sweep)

- P0.1 spec ×4 (1ae394dc…6ffdbcb6, Properties.lean): c1/d1/t1/r1
  holds_fate_iff — o placeholder `: True` do wrapper virou teorema
  real; loops reais provados por spec_decr_nat; ladder 142→146/136→132.
- Fix de motor fora de fatia (052d151e): deadlock do 1º put async sob
  disk pressure soft (reclaim do wal-held lock-free).
- P0.2 modelo ×4 (52e7296e…26f3e463): d1_modelo (D1Modelo.lean),
  r1_modelo (LsmR1.lean), t1_modelo (T1Modelo.lean), c1_modelo
  (C1Modelo.lean) — ramos construtivos carregando a igualdade
  habilitante; 146→150/132→128. Planta r1_modelo achou bug REAL:
  SST v5 vazio reaberto panicava fail_stop com corrupção inventada —
  fix 30e572db (materialize_entries devolve vazio; as 4 falhas
  pré-existentes do suite sst falham igual no HEAD anterior).
- P1.1 fate ×2 (bb22b756+335e78ef): put_ok_fate_iff (cadeia honesta
  wal_append→wal_sync→wal_ack) e c1_advance_commit_fate_iff
  (maioria); 150→152/128→126.
- P1.2 compose espinha→coroa (8d29e2fc): ComposeProductCrown.lean
  (24ª compose lib) — sobre todo ledger spine_reach do 0214, as duas
  pernas JUNTAS: d1_modelo = ok true (cita a iff; corpo extraído NÃO
  reaberto; perna geminada wa_d1_modelo_fate_iff em WriteAck.lean
  resolve o choque de import das cópias geradas) e d1_holds = ok
  true (perna P0.1). Twin product_crown_kernel.rs (2 testes) +
  planta DST on live verde. SEM TSV (não é par único).
- P2.1 http ×6 (4fdec668…4a4d138d): is_bearer_scheme,
  is_non_bearer_auth_scheme, normalize_http_method, ascii_lower,
  ascii_upper, authorization_matches — 152→158/126→120. O loop de
  scan do Authorization fechou nos DOIS sentidos (hit/end/cont);
  técnica nova registrada: o `let (k, v) := (k, v)` elaborado reduz
  só por `conv => lhs; whnf`; ∨ é associativo à direita em Lean 4
  (rcases e construtores anônimos aninhados explicitamente).
- P2.2 sweep (c130482e): worktree /Users/paulo/software/pedradb-
  r0215-sweep @ 4a4d138d DENTRO de software/ — .lake (7,4G) primado
  por clone APFS copy-on-write (13s; build completo replayed em 7s,
  1941 jobs) — depth-floor GREEN (extract=120, atom=158, close=6,
  count=7, data_fate=0), inventory terminal, twins bound,
  test_proof_vs_campaign ok, extracts 64 libs + 24 compose exit 0,
  sorry 0 nos 8 wrappers da rodada; nota datada EXTRACT.md; RFC-0215
  Status done; worktree removido.
- Auditoria final round 9 (HEAD 0dff77c2): 1 teorema/commit em TODAS
  as 16 promoções (compose P1.2 fora da régua por-átomo, veredito/
  sweep = 0); NENHUM arquivo dela no range faeb1f98..0dff77c2;
  admissions 3/3 verdes no HEAD (claim_media_durable_refused_after_
  fsync_ok, forall_schedules_admitted_on_live_group_is_not_ok,
  claim_lock_interleavings_refused_after_put — 1 passed cada);
  depth-floor GREEN no HEAD; extracts --required exit 0 no HEAD.
  Capturas {SCRATCH}/r0215_audit_{theorem_commit,head,admissions,
  extracts}.txt + r0215_sweep_{depth,inventory,twin_contracts,
  campaign,extracts,sorry}.txt + r0215_candidates.txt.
- Sucessora 0dff77c2: RFC-0216 — superfície de parse HTTP no degrau
  átomo. Medição ao vivo (candidates.py @ c130482e): TODOS os boards
  zerados (script 0/17, compose 0/17, concorrência 0/5, scale 0/3,
  produto 0, SA none, A/B/D/C vazios); leftover_next caiu no fallback
  P1.5 (trampolim db.rs/concurrent.rs congelado — mesma non-goal);
  cartoon 4 TODO em Montanha (skip). Restam 120 no extrato; o maior
  bloco coerente é parse HTTP: cl ×4 (Cl.lean: keep_body_without_cl,
  invalid_cl_as_zero, content_length_repeat_ok,
  short_body_vs_cl_is_error) + fail_closed ×1 (FailClosed.lean:
  parse_error_writes_status) + form ×7 (Form.lean) + path ×8
  (Path.lean) = 20 pares → família http 27/27 em átomo, floor_atom
  158→178, floor_extract 120→100.
- PRÓXIMO ROUND: implementar 0216 P0.1 (cl ×4 ao átomo em Cl.lean).
- Fronteira respeitada: arquivos sujos dela intactos
  (caminho-sel4.md, serial-fillin10.log.prevstate, rfc0029 stdout.json,
  __pycache__); nenhum stash tocado; diário NÃO commitado.

## Fire 808 (auditoria-adversarial-remediacao) — 2026-09-24
Auditoria adversarial e remediação integral da verificação formal (RFC-0259 / RFC-0260).
- Redução de axiomas Lean 4: 290 → 265 axiomas (-25 axiomas não modelados convertidos em `def`s funcionais verificadas em Auth, Bloom, Cf, FailClosed, Form, GroupWindow, Key, Leveling, Path, Scale, World, WriteCycle).
- Teto de axiomas travado: `scripts/ratchet/lean_axioms_ceiling.json` (265) e catálogo gerado `scripts/ratchet/lean_axioms_catalog.tsv`.
- Fechamento de sorries ocultos: 3 sorries em `formal/aeneas/lean/WriteCycleKernel.lean` eliminados.
- Novo gate: `scripts/check_lean_sorries_and_axioms.py` varrendo todos os 197 arquivos `.lean` (0 sorries em toda a árvore).
- Modelo de concorrência Stateright: `crates/pedradb-core/tests/write_group_model.rs` expandido para 5 clientes, step budget = 8 (fórmula N+3 descoberta e comprovada), verificando `Inv-committed-bounded-by-publish`, `Inv-monotonic-commit` e `Inv-no-gap` em 7.087 estados e 17.584 transições.
- BMC Kani: harness `recover_record_is_unconditional_keep` em `crates/pedradb-core/src/wal/recover_kernel.rs` provando conservação de registros válidos sobre espaço 2^64-1.
- Teoremas semânticos Compose M2 & POSIX: `wal_ticket_chain_disjoint`, `wal_commit_durable_prefix_preserved`, `posix_rc_refused_of_nonzero`.
- Aprendizados persistidos em `findings/2026-09-24-pos-adversarial-verification-learnings/aprendizados.md`.
- PRÓXIMO: continuar expansão de redução de axiomas de stdlib (< 200) e ratchets de cobertura Kani.

## Fire 809 (rfc0261-rumo-a-zero-axiomas-expansao-stateright-kani) — 2026-09-24
Execução e avanço dos P0, P1, P2 sob a égide do RFC-0261.
- Redução de axiomas Lean 4: 265 → 258 axiomas (-7 axiomas eliminados através de `def` funcional de `core.option.Option.Insts.CoreCmpPartialEqOption.eq` em AuthKernel, ClientAxisKernel, FailClosedKernel, LsmR1Kernel, ProductCrownKernel, PropertiesKernel, WriteCycleKernel).
- Teto travado: `scripts/ratchet/lean_axioms_ceiling.json` em 258; catálogo gerado `lean_axioms_catalog.tsv` com 258 entradas.
- Stateright: `write_group_model.rs` expandido para incluir Snapshot Readers concorrentes observando store MVCC multichave, provando `Inv-atomic-batch-visibility` e `Inv-no-future-read` além dos invariantes de commit e liveness.
- Kani BMC: adicionados harnesses formais em `lookup_kernel.rs` (`kani_point_cache_validity_strict_invalidation` e `kani_point_tombstone_shadowing_iff`) sob `#[cfg(kani)]`.
- Documentação e Ledgers: `lessons_learned.md` criado na raiz do repo espelhando o formato dos repositórios irmãos `../fazenda` e `../janela`, `research/LEDGER.md` atualizado com decisões L51-L55.
- RFC-0261 emitido em `docs/rfc/0261-rumo-a-zero-axiomas-expansao-stateright-kani.md`.
- Gates: 10/10 gates sel4 GREEN, lean_integrity GREEN (0 sorries, 258 axiomas <= 258).


