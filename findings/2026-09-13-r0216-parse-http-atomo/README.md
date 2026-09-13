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
