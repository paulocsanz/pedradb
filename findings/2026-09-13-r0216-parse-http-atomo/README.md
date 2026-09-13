# RFC-0216 — superfície de parse HTTP no degrau átomo (rodada 10)

Promoções átomo da rodada (régua: iff-∀ sobre o corpo extraído no
wrapper inscrito, `floor_atom +1 / floor_extract −1` no mesmo commit,
`check_depth_floor.py` GREEN no HEAD de cada promoção, planta DST
verde antes do commit, exatamente 1 teorema público por commit).

## P0.1 — cl ×4 átomo (1/4)

- **content_length (1/4)**: o gate de corpo sem Content-Length decide
  exatamente na constante honesta extraída — `keep_body_without_cl =
  ok r` é exatamente `r = true` (o kernel sempre mantém o corpo de um
  request sem CL; o mutante AS-IS devolve false e a planta prova o
  dente no HTTP vivo). Planta DST
  `keep_body_without_cl_on_live_http_is_not_ok` (pedradb-http, exit 0,
  1 passed). Gate GREEN: floor_atom 158→159, floor_extract 120→119.

## P0.1 — cl ×4 átomo (2/4)

- **invalid_cl_zero (2/4)**: o gate de CL invalidado decide exatamente
  na constante honesta extraída — `invalid_cl_as_zero = ok r` é
  exatamente `r = false` (o kernel nunca trata CL inválido como zero;
  o mutante AS-IS devolve true e a planta prova o dente). Planta DST
  `invalid_cl_as_zero_on_live_http_is_not_ok` (pedradb-http, exit 0,
  1 passed). Gate GREEN: floor_atom 159→160, floor_extract 119→118.

## P0.1 — cl ×4 átomo (3/4)

- **cl_repeat_conflict (3/4)**: o gate de Content-Length repetido
  decide exatamente na igualdade U64 extraída —
  `content_length_repeat_ok first next = ok r` é exatamente
  `r = decide (first = next)` (o kernel aceita a repetição somente
  quando os dois valores são iguais; o mutante AS-IS aceita sempre).
  Planta DST `content_length_repeat_ok_on_live_http_is_not_ok`
  (pedradb-http, exit 0, 1 passed). Gate GREEN: floor_atom 160→161,
  floor_extract 118→117.

## P0.1 — cl ×4 átomo (4/4, FECHAMENTO P0.1)

- **short_body_vs_cl (4/4, FECHAMENTO P0.1)**: o gate de corpo curto
  contra o Content-Length declarado decide exatamente na comparação
  U64 extraída — `short_body_vs_cl_is_error got declared = ok r` é
  exatamente `r = decide (got < declared)` (o kernel marca erro
  somente quando o corpo recebido é estritamente menor que o
  declarado; o mutante AS-IS nunca marca). Planta DST
  `short_body_vs_cl_is_error_on_live_http_is_not_ok` (pedradb-http,
  exit 0, 1 passed). Axiomas: os 3 padrão do Lean. Fechamento:
  floor_atom 158→162, floor_extract 120→116, gate GREEN no HEAD de
  cada uma das 4 promoções, 1 teorema público por commit.

## P0.2 — fail_closed ×1 átomo (1/1, FECHAMENTO P0)

- **fail_closed (1/1, FECHAMENTO P0)**: o veredito de erro do parse
  decide exatamente na constante honesta extraída —
  `parse_error_writes_status = ok r` é exatamente `r = true` (todo
  erro de parse escreve uma status line, nunca derruba o socket em
  silêncio; o mutante AS-IS devolve false). Planta DST
  `parse_error_writes_status_on_live_http_is_not_ok` (pedradb-http,
  exit 0, 1 passed). Axiomas: os 3 padrão do Lean. Gate GREEN:
  floor_atom 162→163, floor_extract 116→115. P0 completo: todo
  veredito de admissão de request (corpo + erro) em teorema.

## P1.1 — form ×4 átomo (1/4)

- **form_plus_byte (1/4)**: a decodificação de byte de form decide
  exatamente no `if` extraído — `+` (43) vira espaço (32), qualquer
  outro byte é ele mesmo; cada ramo carrega a igualdade habilitante.
  O mutante AS-IS não converte. Planta DST
  `plus_is_space_before_percent` (pedradb-http, exit 0, 1 passed).
  Gate GREEN: floor_atom 163→164, floor_extract 115→114.

## P1.1 — form ×4 átomo (2/4)

- **plus_before_percent (2/4)**: a flag de ordenação `'+'`-antes-
  `'%`'` decide exatamente na constante honesta extraída — o decoder
  trata o `+` antes do escape de percent; o AS-IS não distingue.
  Planta DST `plus_order_flag_discriminates_as_is` (pedradb-http,
  exit 0, 1 passed). Gate GREEN: floor_atom 164→165,
  floor_extract 114→113.

## P1.1 — form ×4 átomo (3/4)

- **from_hex (3/4)**: o if-tree de 15 folhas extraído espelha os
  3 blocos ASCII hex (0-9, a-f, A-F) com os offsets `-48/-97/-65`
  e o `+10`; `r=none` ⇔ fora dos blocos. Achado da prova: em
  `∃ i i1, A ∧ B ∧ C` os binders vêm antes das provas no pattern
  rintro (`i, i1, hi, hi1`, não `i, hi, i1, hi1`). Planta DST
  `hex_letters_decode_as_is_does_not` (pedradb-http, exit 0,
  1 passed). Gate GREEN: floor_atom 165→166, floor_extract 113→112.

## P1.1 — form ×4 átomo (4/4 — slice fechado)

- **form_decode (4/4)**: primeiro LOOP da família fechado. O fate do
  output inteiro é a cadeia citada `DecodeFate` (fuel = bytes
  restantes): cada `cont` é exatamente um passo do corpo extraído
  com índice estritamente crescente e ≤ len (inversão das 11 folhas:
  `+`→32, `%` com 0/1/2 hex válidos avança 1, e o `+3` só ocorre com
  DOIS hex válidos sob guarda `i+2 < len`); `done` só em `i = len`
  com o out intacto. Indução no combustível com `loop.eq_def`.
  Armadilhas: `0#usize` no statement trava a elaboração quando há
  `→` interno (pi postponado — o `intro` falha); ∀-binders diretos
  (estilo Auth) elaboram. Coerções `↑(2#usize)`/`↑(3#usize)` são
  opacas ao omega — precisa `have hXv : ↑iX = ↑i + N := hX.2.1`.
  Planta DST `form_decode_on_live_http_is_not_ok` (pedradb-http,
  exit 0, 1 passed). Gate GREEN: floor_atom 166→167,
  floor_extract 112→111.

## P1.2 — form ×3 átomo (1/3)

- **query_u64_conflict (1/3)**: o conflito u64 é exatamente a
  desigualdade decidida dos dois lados (`r = (a != b)`); o mutante
  AS-IS sempre responde "sem conflito". Prova direta por
  `Result.ok.inj` nas duas direções — sem loop. Planta DST
  `f155_query_conflict` (pedradb-http, exit 0, 1 passed). Gate
  GREEN: floor_atom 167→168, floor_extract 111→110.

## P1.2 — form ×3 átomo (2/3)

- **query_part_is_bare_name (2/3)**: veredito "parte é nome puro"
  é exatamente a cadeia citada — parte vazia ⇒ falso, parte com
  `=` ⇒ falso, senão o decode da parte comparado byte a byte com a
  chave via o eq extraído. Achado da prova: após `simp only
  [bind_tc_ok]` a condição do if já reduz para literal (`if True`),
  então `rw [if_pos rfl]` NÃO casa — `simp` fecha; o `if_neg (by
  simp)` nas condições falsas casa normal. Planta DST
  `f155_query_conflict` (pedradb-http, exit 0, 1 passed). Gate
  GREEN: floor_atom 168→169, floor_extract 110→109.

## P1.2 — form ×3 átomo (3/3 — slice fechado)

- **query_values_conflict (3/3)**: segundo LOOP da família fechado.
  O conflito de valores repetidos é a cadeia citada: menos de 2
  valores ⇒ falso; senão o primeiro é fixado pelo index e o scan
  `ValuesFate` (fuel = len−1, começa em 1) decide — cada igual
  avança exatamente 1 (`cont i'` com `i < i' ≤ len`), o primeiro
  diferente responde true, varrer até o fim responde false. Molde
  do form_decode reusado (body_cases + body_at_end + indução em
  combustível). Armadilhas novas: o corpo do loop é uma lambda
  DIRETA (`fun i1 => body values first i1`), então o `dsimp only`
  pós-`rw [loop.eq_def]` NÃO progride (já beta-reduzido) — só nos
  ramos; o `cases hB :` substitui o body no `h`, então a
  contradição do zero/cont fecha direto no `hB` (ok (done false) =
  ok (cont st)), não no `h`; `subst hii : i' = i2` elimina o `i2`
 (var local do rcases) — usar `i'` depois. Coerções: ponte iff
  `hgate` entre `Slice.len < 2#usize` e `.length < 2` (via
  `Slice.len_val` + `UScalar.lt_equiv`) unifica os átomos do omega.
  Planta DST `f155_query_conflict` (pedradb-http, exit 0,
  1 passed). Gate GREEN: floor_atom 169→170, floor_extract
  109→108.

## P2.1 — path ×8 átomo (1/8)

- **strip_authority_for_routing (1/8)**: o corpo extraído é
  `fun b => ok b` — repassa exatamente a bandeira de
  forma-authority; o AS-IS dente (`ok false` constante) já estava
  provado no Path.lean. Copiado o `bind_ok_inv` privado para o
  Path.lean (file-scoped, mesmo molde do Form.lean). Planta DST
  `strip_flag` (pedradb-http, exit 0, 1 passed). Gate GREEN:
  floor_atom 170→171, floor_extract 108→107.

## P2.1 — path ×8 átomo (2/8)

- **strip_uri_fragment (2/8)**: o fragmento é descartado
  exatamente pelo `split_once` no `#` — sem `#` a target volta
  inteira, com `#` fica o prefixo `a`. Molde simples de match sobre
  Option com par: `cases o` + `obtain ⟨a, snd⟩ := pair` no some.
  No reverso, `simp only [bind_tc_ok]` já reduz o match no
  constructor (não precisa de `rfl` — "no goals"). Planta DST
  `origin_strips_absolute_and_network` (pedradb-http, exit 0,
  1 passed). Gate GREEN: floor_atom 171→172, floor_extract
  107→106.

## P2.1 — path ×8 átomo (3/8)

- **path_after_authority (3/8)**: o path depois da autoridade é o
  primeiro `/` em diante — sem `/` a resposta é a raiz `toStr "/"`
  (literal citável via `path_after_authority._proof_1`), com `/` é
  o `index ... { start := i }` extraído. No some, o `dsimp only`
  já entrega `hval : index ... = ok r` pronto (não envolver em
  `Result.ok.inj`). Planta DST
  `authority_atoms_discriminate_as_is` (pedradb-http, exit 0,
  1 passed). Gate GREEN: floor_atom 172→173, floor_extract
  106→105.

## P2.1 — path ×8 átomo (4/8)

- **strip_http_authority (4/8)**: a autoridade HTTP é descartada
  exatamente pelo `strip_http_authority_rest` — sem `//` prefixo a
  resposta é none; com `//` é o `path_after_authority` do rest.
  Duplo bind no some (rest → p); o `bind_ok_inv` do segundo bind
  já entrega `ok (some p) = ok r` beta-reduzido (o `dsimp` ali é
  no-progress). Planta DST
  `authority_atoms_discriminate_as_is` (pedradb-http, exit 0,
  1 passed). Gate GREEN: floor_atom 173→174, floor_extract
  105→104.
