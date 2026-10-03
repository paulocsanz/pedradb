# Security Policy

This policy covers the `pedradb-*` crates maintained in this repository
(`pedradb-core`, `pedradb-posix`, `pedradb-io-uring`, and friends).

## Supported versions

PedraDB is pre-1.0 software with no backport branches. Security fixes land on
`main` and ship in the next release.

| Version | Supported |
|---------|-----------|
| latest release / `main` | ✅ |
| anything older | ❌ — upgrade to the latest release |

## Reporting a vulnerability

**Do not open a public issue, PR, or discussion for a suspected vulnerability.**

Use GitHub's private vulnerability reporting instead — the **Report a
vulnerability** button in this repo's
[Security tab](https://github.com/paulocsanz/pedradb/security/advisories/new).
Reports are encrypted in transit and visible only to the maintainer.

Please include, as far as you can:

- the crate name and exact version or commit (`git rev-parse HEAD` if a git
  dependency),
- platform and filesystem details (OS, arch, disk/FS type, and whether the
  io_uring or posix I/O path was in use),
- a minimal reproducer — ideally a script or a database directory that
  triggers the issue,
- observed impact (crash, corrupted state, data loss, memory-safety symptom).

If you are unsure whether something is a security issue, err on the side of
caution and report it privately. Ordinary bugs (no security impact) should go
through the regular public issue tracker.

## Threat model

PedraDB is an **embedded, in-process library**. There is no server, no
network protocol, no authentication, and no privilege boundary inside the
engine: the embedding application decides which directories to open, and
whoever can open a database directory already owns that data.

### In scope

- **Memory-safety violations** reachable from data PedraDB parses — e.g. an
  out-of-bounds access, slice aliasing violation, or other undefined behavior
  triggered by opening or replaying a corrupted or attacker-crafted database
  directory, WAL, or segment file. This explicitly includes the `unsafe`
  blocks in the POSIX, io_uring, and WAL kernels.
- **Violations of the crash-safety contract.** Durability and atomicity are
  the product: a commit that returns `Ok` without its WAL record being
  durably on disk (`fdatasync` before `Ok`), or a crash mid-write that leaves
  a partial or torn transaction visible after replay, is treated as a
  security-relevant data-loss/data-corruption issue.
- **Violations of the documented ACID guarantees** that can corrupt state or
  expose uncommitted data across transactions.

### Out of scope (still welcome as regular bug reports)

- Panics, OOM, or unbounded resource consumption from adversarial inputs
  when there is no memory-safety violation (denial-of-service class).
- Performance or resource behavior of legitimate workloads.
- An attacker who can already write to the database directory or execute
  code in the host process — they control the engine by definition.
- Physical access to storage media, hardware, or firmware attacks.
- Issues only reproducible on unsupported platforms or configurations.
  PedraDB is developed and CI-tested on Linux (x86_64 and aarch64; the
  io_uring path is Linux-only); macOS is best-effort; everything else is
  unsupported.
- Bugs in forks or in downstream products that embed PedraDB — report those
  to the respective projects.

## Response and disclosure

- Reports are acknowledged within **7 days**, and you will be kept updated
  through triage, fix, and release.
- Fixes target the next release; there are no patch branches before 1.0.
- Disclosure is coordinated: we ask that you not publish details until a
  fixed release is out. The default window is **90 days** from
  acknowledgment, shorter if the issue is already public or being exploited.
- If a report is confirmed as a security vulnerability, we will publish a
  GitHub security advisory and request a CVE through GitHub's CNA. Please
  do not request a CVE independently.
- Reporters are credited in the advisory unless they prefer otherwise.

There is no bug bounty program at this time.
