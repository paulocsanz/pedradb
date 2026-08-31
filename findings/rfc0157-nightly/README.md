# RFC-0157 P2.3 — runner noturno de verificação (registro em `findings/`)

Campanha noturna: **TCP REAL K seeds em paralelo** + **sweeps PCT d=3/d=4**,
cada execução registrada neste diretório com material fresco. É âncora de
evidência, não prova de ∀ nada.

## Comandos (exatos)

Data `YYYY-MM-DD`, prefixo de seed da noite `0x0157_N0` + índice (escolha um
prefixo novo por noite; nunca reutilize `0x0156/0157/0158_1E28`,
`0x0157_C0xx` nem o prefixo de uma noite anterior):

```bash
DATE=2026-08-30            # exemplo
NIGHT=findings/rfc0157-nightly/$DATE

# 1) TCP REAL K-paralela (K default 8; gates: exit 0, fator < 2.0,
#    todas as seeds com fingerprint limpo)
RFC0157_SEED_PREFIX=0x0157_N0 \
RFC0157_OUT=$NIGHT \
RFC0157_TITLE="RFC-0157 P2.3 — nightly REAL TCP campaign registration $DATE" \
  scripts/rfc0157_tcp_campaign.sh 8 | tee $NIGHT.console.txt

# 2) PCT d=3 (planta cadeia-3; espera: d=2 0/256, d=3 >=1/16384)
cargo test -p pedradb-world --features pct --lib \
  planted_chain3_found_by_pct_d3 -- --nocapture 2>&1 | tee $NIGHT/pct_d3.txt

# 3) PCT d=4 noturno (headroom de profundidade; taxa e custo registrados)
cargo test -p pedradb-world --features pct --lib \
  planted_chain3_pct_d4_nightly -- --nocapture 2>&1 | tee $NIGHT/pct_d4.txt
```

## Padrão de registro

Cada noite é um diretório `findings/rfc0157-nightly/<data>/` contendo:

- `README.md` — gerado pelo script da campanha: cabeçalho (data, host, K,
  portas, fator wall/solo), tabela por seed (seed / tentativas / segundos /
  fingerprint) e seção **Piso**.
- `logs/seed_<i>.txt` — log bruto por seed (append entre tentativas;
  `_fingerprint_` extraído da última linha `cluster_real `).
- `pct_d3.txt` / `pct_d4.txt` — saída capturada dos sweeps (comando +
  contagens + tempo do sweep d=4).
- O prefixo de seed usado deve aparecer no `README.md` da noite (a tabela de
  seeds o registra) — seeds são evidência só quando frescas.

Gates da noite (todos devem fechar verde):

1. campanha TCP: `campaign: OK` (exit 0), fator < 2.0, `napply=1` em toda seed;
2. PCT d=3: teste verde (0 falhas, 0 contraexemplos fora da planta);
3. PCT d=4: teste verde; registrar taxa (`found h/16384`) e custo (wall do
   sweep impresso pelo teste). Se a máquina da noite não comportar o sweep
   d=4 inteiro, registrar o limite real medido (seeds executadas, tempo) em
   `pct_d4.txt` — não fabricar cobertura.

## Armadilha do launcher (2026-08-31, pescada antes do r1 disparar)

Os guardas PCT vivem em `#[cfg(feature = "pct")] mod pct_concurrent` —
**sem `--features pct` o filtro casa 0 testes e o cargo sai 0 (verde
vazio)**. Um launcher que copie só o nome do teste reproduz a classe
fail-open do colapso de seeds. Todo launcher noturno deve usar os comandos
acima verbatim (`--features pct --lib`) e conferir no output
`test result: ok. 1 passed` — 0 matched = falha, não sucesso. O launcher
r1 foi re-armado com essa checagem mecânica (grep do `1 passed`; ausência
→ exit 99).

## Piso (o que a noite NÃO prova)

- K seeds com fingerprint limpo é **evidência**, não ∀ TCP (R-swarm-real).
- retry ≤3 é harness; liveness não é admitida (R-es).
- PCT d=3/d=4 é amostragem; o runner exaustivo do P1.3 cobre todo o espaço
  só para N≤3, e o caminho vivo com disco continua lower-bound
  (R-pct / R-group-glue / R-glue).
- Noite verde não é "sem bugs": o `never_floor` segue inteiro
  (R-cpu, R-rustc, R-verus, R-crc, R-deps, R-extract).
