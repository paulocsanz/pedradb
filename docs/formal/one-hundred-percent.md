# O que “provar 100%” exigiria

**Updated:** 2026-08-23  
**Programa:** [RFC-0053](../rfc/0053-ironfleet-years.md)  
**Frase canónica:** kernel K ⊨ spec S, relativo a axiomas A — nunca “não há bugs no Pedra”.

---

## 0. 100% de quê

“100% do Pedra” sem TCB é vazio: rustc, LLVM, CPU, Linux, `fsync`, CRC, `lz4`, o loop de I/O, o hardware. seL4 (~10k C, ~200k Isabelle) ainda deixa hardware+boot+spec de fora. IronRSL deixou o **loop principal** no TCB.

A única leitura útil:

```text
100%  =  100% das transições que decidem destino de dados
         estão num kernel que produção chama
         e cada kernel tem prova ∀
         e cada caller refina o kernel
         e a composição (host + crash + net) está no mesmo spec
         RELATIVO a um TCB escrito.
```

Abaixo, o que falta para *essa* frase. Números desta árvore (2026-08-23):

| Objecto | LOC (aprox.) |
|---------|----------------|
| `db.rs` | 14 392 |
| `ConcurrentDb` | 4 846 |
| `pedradb-store` `lib.rs` | 10 805 |
| `pedradb-raft` `lib.rs` | 1 767 |
| Gémeos Verus | ~3 250 |
| IronRSL (referência) | 5 114 impl + 39 253 prova |

Só `db.rs` já é ~3× a impl que o IronFleet verificou.

---

## 1. TCB — o que 100% **nunca** inclui

Estas peças ou ficam axioma, ou precisas de *outro* projecto (CompCert, seL4, Ironclad):

| Camada | Para “100%” absoluto | Pedra |
|--------|----------------------|-------|
| CPU / microcódigo | modelo de ISA + pipe | nunca |
| Linux / `fsync` / io_uring | prova do kernel + contrato POSIX | DST + det_io + TCG (RFC-0052) |
| rustc / LLVM | CompCert-class | TCB |
| Verus + Z3 | “verificador verificado” | TCB |
| CRC32C | “não se forja um CRC válido” | axioma VeriBetrKV |
| `lz4_flex`, `parking_lot`, `bytes` | stubs verificados ou extract | TCB de facto |
| `unsafe` posix / uring / capi | prova de SAFETY.md ou Miri+ASan como rampa | ilhas; não ∀ |

**Precisamos:** uma página TCB que o CI recusa se crescer em silêncio (RFC-0053 P0.4). Sem isto “100%” muda todas as semanas.

### Axiomas de liveness (declarados, não provados) — RFC-0056 P2.2

Liveness nunca é teorema. As propriedades de eventualidade do modelo do
node inteiro (`crates/pedradb-raft/tests/tcp_node_model.rs`) valem **só**
sob três axiomas, nomeados no modelo e aqui:

- **ES-1 (adversário finito / sincronia eventual):** o ambiente é
  adversário por ≤ `ES_BOUND` passos — crash-restarts finitos, sem
  partição infinita. Depois disso o persist nunca falha.
- **ES-2 (drain interno):** sincronizado, o loop de apply corre até à
  quiescência antes do próximo frame ser lido (progresso interno não é
  escalonável para fora).
- **ES-3 (candidato vivo em retry):** sincronizado, enquanto o node não
  tiver concedido voto, o RequestVote de um candidato vivo continua a
  ser entrego.

Sem os axiomas o modelo **refuta** ambas as propriedades de eventualidade;
sem ES-3 só a eleição cai (cada axioma é load-bearing). Atacar estes
axiomas é o item 12, para sempre.

---

## 2. Checklist — 100% relativo a esse TCB

Cada linha é **necessária**. Nenhuma sozinha chega.
**Estado final (2026-08-23, RFC-0056 fechado):** [one-hundred-percent-report.md](one-hundred-percent-report.md).

| # | O que tem de ser verdade | Hoje | Falta |
|---|---------------------------|------|-------|
| **1** | Todo `if` de destino de dados é um `fn` puro; **produção chama** | **verde**: 35 kernels (voto, AE, commit, TX, lease, WAL recover, prefix, bloom, pack, flush, compact, MANIFEST, vlog GC, 2PC glue, …); freeze recusa kernels não registados | só o group-commit/`ConcurrentDb` (item 7) |
| **2** | Cada um desses `fn` tem prova ∀ (Verus) | **verde**: 44 pares, 40 twins, twin:kernel ≈ 0.65:1, zero `sorry` | — |
| **3** | 2ª máquina (Aeneas→Lean) no **extract**, não gémeo à mão | **verde**: 8 extracts drift-stamped (voto, isolated, bloom, AE, commit, reopen, apply, wal-recover) | — |
| **4** | Caller refina: `Grant ⇒ persist Ok`, `ACK ⇒ majority` | **verde**: F15/F11/F16 kernels + refinamento verificado no loop TCP (8/8) | — |
| **5** | Composição: um spec de **dicionário + crash** (acked prefix sobrevive) | **verde**: `dictionary_link` (put→…→get, hipóteses nomeadas) + DST e2e no CI | — |
| **6** | Host loop é máquina de estados (redução: passo atómico) | **verde**: `compose_model` + `tcp_node_model` (node inteiro; kernels de produção no passo) | — |
| **7** | Concorrência: ou redução a (6), ou lógica concorrente | `ConcurrentDb` real; PCT **não** ligado; TSan opcional | **aberto (gate RFC-0051)**; π/VerusSync quando o gate abrir |
| **8** | Liveness (eleição eventualmente, se quórum vivo) | **verde relativo aos axiomas**: `Property::eventually` no modelo do node; ES-1/2/3 declarados (§1); refutado sem axiomas | atacar os axiomas (item 12) |
| **9** | I/O é transição (Hance): crash = reset + torn unacked | **verde**: `FailingEnv`/`RecordingEnv` por todo o path; zero `Db<StdEnv>` hardcode em API de biblioteca; journal+index injectáveis | — |
| **10** | Glue não-kernel é **zero** ou no TCB à vista | **à vista**: 7.723 kernel / 5.022 twin / 39.793 handler LOC; TCB freeze no CI | zero-glue continua a trajecto |
| **11** | Twin drift = CI vermelho | **verde**: lint (entry+handlers) + clones (tokens) + SOURCE sha256 + TCB freeze; negativos em cada wave | — |
| **12** | Axiomas atacados para sempre (não “fechados”) | World sibling, det_io residual, **ES-1/2/3 declarados (§1)** | RFC-0050 World in-tree; 0052 Miri/TCG — contínuo por definição |

Fecho 2026-08-23 (RFC-0056): **1–6, 8, 9, 11 verdes; 7 aberto por gate (RFC-0051); 10 verde no
braço “à vista”; 12 contínuo**. Isso é o máximo que seL4/IronFleet chamam de sistema
verificado. **Não é 100% do universo.**

---

## 3. O que *não* precisamos (e come o “100%”)

- Reescrever em Dafny/Coq para dizer “o binário é o termo” — produção é Rust.  
- Provar `Content-Length` antes de P40 / WAL / `handle_*`.  
- Provar o Linux.  
- ∀ interleavings do `ConcurrentDb` no ano 1.  
- “100% de cobertura de testes” (RFC-0004: profundidade, não %).  
- Paridade de CPU-hours com o Sim2 do FDB.

---

## 4. Ordem se o objectivo é essa frase

1. P40 (extract do voto = 2ª máquina de verdade).  
2. Spec crash-dictionary + dentes DST.  
3. Inventário CI dos kernels.  
4. Refinar callers Raft.  
5. Compor Stateright.  
6. Extrair flush/MANIFEST/compact **como kernels**, não como `db.rs` ∀.  
7. Só então o path `Db` single-thread no spec de crash.  
8. Threads por último.

Isto *era* o horizonte Y1–Y3 do RFC-0053 (**shipped** 2026-08-23). A entrega dos restantes itens 1–11 é o [RFC-0056](../rfc/0056-one-hundred-percent-delivery.md). “100%” não é um slice extra — é o TCB deixar de crescer e o glue em (10) tender a zero.
