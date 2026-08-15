# Process — ponte

A norma é [`QUALIDADE.md`](QUALIDADE.md).
O procedimento de sessão é [`AGENTS.md`](AGENTS.md).
O próximo passo é [`PLANO.md`](PLANO.md).
A auditoria é a skill `/audit-qualidade`.

Estados do `catalog.tsv` (`listed` / `have-pdf` / `ficha` / `blocked` /
`demoted`) continuam válidos. `ficha` **só** depois de score ≥ D3.

Fetch:

```bash
./research/scripts/fetch-one.sh R010 https://www.usenix.org/system/files/fast21-dong.pdf Dong_2021_RocksExperience
```
