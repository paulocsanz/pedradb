# Post-extract patch for kernel 'scan' — moved verbatim from
# scripts/aeneas_scan.sh. Applied by `cargo xtask proof scan` after the
# Charon+Aeneas extract; argv (paths resolved by xtask): ['$OUT/lean/ScanKernel.lean']
import sys
p = sys.argv[1]
src = open(p, encoding="utf-8").read()
old = (
    "  (tupled_args : ((Slice Std.U8) × (Slice Std.U8))) :\n"
    "  Result (Bool × scan_kernel.scan_reads_file.closure)\n"
    "  := do\n"
    "  sorry\n"
)
new = (
    "  (tupled_args : ((Slice Std.U8) × (Slice Std.U8))) :\n"
    "  Result (Bool × scan_kernel.scan_reads_file.closure)\n"
    "  := do\n"
    "  let (t_start, t_end) := tupled_args\n"
    "  let (start, end1) := c\n"
    "  let b ← scan_kernel.tombstone_reaches_window t_start t_end start end1\n"
    "  ok (b, c)\n"
)
if old in src:
    open(p, "w", encoding="utf-8").write(src.replace(old, new, 1))
    print("      patched scan_reads_file closure call_mut")
elif "let b ← scan_kernel.tombstone_reaches_window t_start t_end start end1" in src:
    print("      scan_reads_file closure already patched")
else:
    sys.exit("scan_reads_file closure patch target not found")
if "sorry" in src.replace(old, new, 1) if old in src else src:
    # re-read after write
    pass
src2 = open(p, encoding="utf-8").read()
if "\bsorry\b" in src2 or "\nsorry\n" in src2 or src2.endswith("sorry\n"):
    sys.exit("ScanKernel.lean still contains sorry")
