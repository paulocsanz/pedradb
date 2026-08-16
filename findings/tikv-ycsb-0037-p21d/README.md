# RFC-0037 dual-mem flush worker

p21c: apply 3332 vs Rocks 2747 (Rocks apply slow).
p21d (official): apply 3035 vs Rocks 5620 clean (max 4.8ms) = 0.54.
Scan 0.45–0.54. WAL fdatasync before Ok.

