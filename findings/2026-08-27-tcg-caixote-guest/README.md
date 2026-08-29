# RFC-0063 P2.3 — guest TCG real no caixote (`PEDRA_QEMU_SSH`)

**Date:** 2026-08-27
**Guest:** `pedra-tcg-guest` (project `pedradb-dst`, region `brasil`, `rust:1-bookworm` + sshd)
**Não** é guest inventado localmente. **Não** é bench (wall-clock forbidden).

O hook `PEDRA_QEMU_SSH` aponta ao SSH público do serviço (`tcp.caixote.net`,
porta TCP alocada). Chave local `~/.ssh/pedra_tcg_guest` (não no git).

```
export PEDRA_QEMU_SSH='-i ~/.ssh/pedra_tcg_guest -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o IdentitiesOnly=yes -p 30521 root@tcp.caixote.net'
scripts/tcg_guest_status.sh
# C2.2=guest_reachable
```

CI sem esta env continua `C2.2=residual_no_guest` (honesto). `TCG_REQUIRED=1`
continua fail-closed.

## seed=42 `world_smoke` — mesmo `trace_hash`, `silent_wrong=0`

`t=166` é tick lógico, não parede.

| Caixa | VMM | hash |
|---|---|---|
| Darwin arm64 native (release) | host | `48b32d03bae5f932` |
| caixote Linux x86_64 (`pedra-tcg-guest`) | `qemu` | `48b32d03bae5f932` |
| caixote Linux x86_64 (`pedra-tcg-guest`) | `deterministic` (TCG single-thread) | `48b32d03bae5f932` |
| Alpine initramfs `qemu-system-x86_64 -accel tcg -smp 1 -icount shift=6,sleep=off` | Pedra RFC-0052 P2.1 | `48b32d03bae5f932` |

Fingerprint comum: `puts_ok=0 puts_err=6 gets_ok=1 dcs_ok=0 silent_wrong=0 events=184 net_sent=8 dropped=0 rpc=0 disk_arms=3 trip=0 t=166`.

`C2.1=native_eq_guest` (`scripts/tcg_world_smoke.sh`).
`C2.2=guest_reachable` (`scripts/tcg_guest_status.sh` + `PEDRA_QEMU_SSH`).

## Notas de provision

- Ubuntu template `tpl_ubuntu` + `vmm_runtime=deterministic` **falhou na hora** com `disk_mb=1024` (cloudimg noble). Não inventar disco.
- Recriado o **mesmo** nome `pedra-tcg-guest` como container `rust:1-bookworm`, `disk_mb=20480`, sshd no PID 1. Primeiro boot em `qemu` (apt não-TCG); depois `vmm_runtime=deterministic` (RFC 0164).
- `set-vmm-runtime` tentou deployment `trigger` fora de `deployments_trigger_check`; o stamp + `redeploy` manual aplicou TCG.
- `caixote service exec` 502 (`invalid HTTP version parsed` no proxy QGA) — SSH é o caminho.
- CLI `caixote project list` / `--project=slug` quebra (`map, expected a sequence`); provision via API com `project_id`.
- Porta TCP muda em redeploy (25836 qemu → 30521 deterministic). Não hard-code em CI.

Artefactos neste dir: `darwin-native.txt`, `caixote-linux-guest.txt` (qemu), `caixote-deterministic-guest.txt` (TCG), `tcg-initramfs-serial.txt`, `tcg_guest_status.txt`, `recreate-running.txt`.

## 2026-08-27 later — same name, new resource

`cnt_d129bee…` elastic/ublk D-state (PID 32751, `/dev/ublkc6`). Recreated **same name** `pedra-tcg-guest` as `cnt_07815c8f8b72458f87f07d654c037cde` (`scfg_c639cbc3…`, `vmm_runtime=deterministic`, `-accel tcg,thread=single`, **no icount**).

Live proof: `caixote service exec … echo qemu_guest_ok` e **C2.1** seed 42 neste cnt:

| caixa | hash |
|---|---|
| Darwin arm64 native (este tree) | `48b32d03bae5f932` |
| caixote `pedra-tcg-guest` TCG single-thread (`cnt_07815c8f…`) | `48b32d03bae5f932` |

Mesmo fingerprint: `silent_wrong=0 events=184 t=166`. TCP `:30521` / `PEDRA_QEMU_SSH` da reincarnação anterior está morto. Detalhe em `recreate-running.txt`.
