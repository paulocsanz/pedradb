# Sinopse: R044 — FoundationDB Record Layer

- **Ficha:** [`../fichamentos/ficha_R044_Chrysafis_RecordLayer.md`](../fichamentos/ficha_R044_Chrysafis_RecordLayer.md)
- **Tier:** D4
- **Tese (com locus):** records/índices/SQL são **layer stateless** sobre um KV com TX já SSI (p. 1–2, 11–12). Índice = projecção KV **na mesma TX** (p. 6); Solr-eventual é o que CloudKit abandonou (Table 1).
- **Número que importa:** biliões de record stores; TX CloudKit ~7 / 36 KB (p50/p99); query ~38.3 keys (~15% overhead); write ~4 keys de índice por record; Fig. 1 maioria dos stores privados < 1 KB; TEXT bunching 11.1→4.9 kB/doc (média 4.7, não 20). Limite FDB 5 s / 10 MB *vaza* para continuations.
- **Para Pedra/Montanha:** L9 confirmado. **L31 SHIP** índices na mesma TX (já `pedradb-index`). **L32 REFUSE** produto RL (protobuf/Cascades/TEXT) no kernel. **L33 MEASURE** VERSION index no fold/CHANGELOG. **L34 REFUSE** atómicos FDB no kernel. SQL continua layer em cima — o próprio paper põe SQL *acima* do RL (p. 12).
- **Não ler isto como:** “clonar o Record Layer Java” nem “Pedra precisa de biliões de tenants protobuf”.
