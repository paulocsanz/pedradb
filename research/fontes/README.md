# Fontes brutas

PDF / HTML / TXT persistidos **no momento da leitura**.

```
fontes/Rxxx_Autor_Ano_Slug.pdf
fontes/Rxxx_Autor_Ano_Slug.txt   # pdftotext; opcional, commitível
```

Já no repo: **não copiar.** Apontar a ficha para
`docs/references/….pdf`.

```bash
./research/scripts/fetch-one.sh R010 https://www.usenix.org/system/files/fast21-dong.pdf Dong_2021_RocksExperience
# opcional
pdftotext -layout research/fontes/R010_Dong_2021_RocksExperience.pdf \
  research/fontes/R010_Dong_2021_RocksExperience.txt
```

PDFs são gitignored. `.txt` pode entrar no git (verificador de citação).
