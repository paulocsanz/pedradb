#!/usr/bin/env bash
# Isolated libc fdatasync p50 on this box (RFC-0041 P0.2). Not F_FULLFSYNC.
set -euo pipefail
OUT="${1:-/dev/stdout}"
python3 - "$OUT" <<'PY'
import ctypes, os, statistics, sys, tempfile, time
libc = ctypes.CDLL(None)
fdatasync = libc.fdatasync
fdatasync.argtypes = [ctypes.c_int]
fdatasync.restype = ctypes.c_int
n = 200
path = tempfile.mktemp(prefix="pedra-fd-")
fd = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
os.write(fd, b"x" * 4096)
samples = []
for _ in range(n):
    os.pwrite(fd, b"y" * 4096, 0)
    t0 = time.perf_counter_ns()
    if fdatasync(fd) != 0:
        raise OSError("fdatasync")
    samples.append((time.perf_counter_ns() - t0) / 1000.0)  # µs
os.close(fd)
os.unlink(path)
samples.sort()
def pct(p):
    return samples[int(round((len(samples) - 1) * p / 100.0))]
text = (
    f"fdatasync_isolated n={n} p50_us={pct(50):.1f} p95_us={pct(95):.1f} "
    f"p99_us={pct(99):.1f} min_us={samples[0]:.1f} max_us={samples[-1]:.1f}\n"
)
sys.stdout.write(text)
if sys.argv[1] != "/dev/stdout":
    open(sys.argv[1], "w").write(text)
PY
