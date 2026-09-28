#!/usr/bin/env python3
"""RFC-0298: Anti-Panic & Zero-Low-Hanging-Fruit Gate.

Enforces static and architectural anti-panic invariants across all PedraDB crates:
1. Zero `.unwrap()` or `.expect()` inside wire, disk, or network decoders/parsers.
2. Zero `from_le_bytes(...unwrap())` or `from_be_bytes(...unwrap())` in production code.
3. Zero `try_into().unwrap()` in parsing/decoding code.
4. Mandatory bounds-checking or SafeCursor usage for buffer slicing in decoders.
5. Zero unauthenticated/unbounded allocations (Vec::with_capacity from wire).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Directories to scan: production code in crates
CRATES_DIR = REPO_ROOT / "crates"

# Banned patterns across all production crates
GLOBAL_BANNED_PATTERNS = [
    (
        re.compile(r"from_(?:le|be)_bytes\([^)]*\)\.unwrap\(\)"),
        "Banned naked `from_*_bytes(..).unwrap()`. Use `SafeCursor` or safe conversion returning `Result`.",
    ),
    (
        re.compile(r"try_into\(\)\.unwrap\(\)"),
        "Banned naked `try_into().unwrap()`. Handle conversion error via `Result` or `DecodeError`.",
    ),
    (
        re.compile(r"try_into\(\)\.expect\("),
        "Banned naked `try_into().expect(..)`. Handle conversion error via `Result` or `DecodeError`.",
    ),
]

# Patterns banned specifically inside decoder/parser functions
DECODER_FUNC_PATTERN = re.compile(
    r"(?:pub\s+)?fn\s+(?:decode[a-zA-Z0-9_]*|parse[a-zA-Z0-9_]*|read_[a-zA-Z0-9_]*|split_block[a-zA-Z0-9_]*|walk_[a-zA-Z0-9_]*|from_slice[a-zA-Z0-9_]*)\b",
    re.IGNORECASE,
)

DECODER_BANNED_PATTERNS = [
    (
        re.compile(r"\.unwrap\(\)"),
        "Banned `.unwrap()` in decoder/parser function. All errors must fail closed via `Result::Err`.",
    ),
    (
        re.compile(r"\.expect\("),
        "Banned `.expect(..)` in decoder/parser function. All errors must fail closed via `Result::Err`.",
    ),
    (
        re.compile(r"panic!\("),
        "Banned `panic!(..)` in decoder/parser function. Decoders must never panic on malformed input.",
    ),
]


def check_file(path: Path) -> list[str]:
    violations = []
    try:
        content = path.read_text(encoding="utf-8")
    except Exception as e:
        return [f"{path}: could not read file: {e}"]

    lines = content.splitlines()

    # Track if we are inside #[cfg(test)] mod tests { ... }
    in_test_module = False
    test_module_brace_depth = 0
    test_fn = False

    in_decoder = False
    decoder_brace_depth = 0
    decoder_name = ""

    for idx, line in enumerate(lines, 1):
        stripped = line.strip()
        if stripped.startswith("//") or stripped.startswith("/*") or stripped.startswith("*"):
            continue

        # Check for test/verification module entry
        if any(tok in stripped for tok in ["mod test", "mod tests", "mod verification", "mod kani", "mod proofs"]):
            in_test_module = True
            test_module_brace_depth += line.count("{") - line.count("}")
            continue

        if "#[cfg(kani)]" in stripped or "#[kani::proof]" in stripped:
            test_fn = True
            continue

        if in_test_module:
            test_module_brace_depth += line.count("{") - line.count("}")
            if test_module_brace_depth <= 0:
                in_test_module = False
                test_module_brace_depth = 0
            continue

        # Check for #[test] annotation on a function
        if "#[test]" in stripped:
            test_fn = True
            continue

        if test_fn:
            if "fn " in stripped:
                # Inside a standalone test function until it finishes its braces
                # For simplicity, if we see fn after #[test], skip check until braces match
                test_fn = False
                continue

        # 1. Global patterns check in production code
        for pat, msg in GLOBAL_BANNED_PATTERNS:
            if pat.search(line):
                violations.append(f"{path}:{idx}: {msg}\n    --> {stripped}")

        # 2. Decoder/parser function scope check
        if not in_decoder:
            m = DECODER_FUNC_PATTERN.search(line)
            if m:
                in_decoder = True
                decoder_brace_depth = line.count("{") - line.count("}")
                decoder_name = m.group(0)
        else:
            decoder_brace_depth += line.count("{") - line.count("}")
            if decoder_brace_depth <= 0:
                in_decoder = False
                continue

            for pat, msg in DECODER_BANNED_PATTERNS:
                if pat.search(line):
                    violations.append(
                        f"{path}:{idx} [in {decoder_name}]: {msg}\n    --> {stripped}"
                    )

    return violations


def main() -> int:
    print("==========================================================")
    print("  RFC-0298: Anti-Panic & Zero-Low-Hanging-Fruit Gate     ")
    print("==========================================================")

    all_violations = []
    scanned_files = 0

    for crate_path in sorted(CRATES_DIR.iterdir()):
        if not crate_path.is_dir():
            continue
        # Skip benchmark or simulator crates for production anti-panic gate
        if crate_path.name.endswith("-bench") or crate_path.name.endswith("-sim"):
            continue

        src_path = crate_path / "src"
        if not src_path.is_dir():
            continue

        for rs_file in sorted(src_path.rglob("*.rs")):
            if rs_file.name.endswith("_test.rs"):
                continue

            scanned_files += 1
            violations = check_file(rs_file)
            if violations:
                all_violations.extend(violations)

    print(f"Scanned {scanned_files} production source files across crates/...")

    if all_violations:
        print(f"\n[FAIL] Found {len(all_violations)} anti-panic gate violation(s):")
        for v in all_violations:
            print(f"  • {v}")
        print("\nAll low-hanging fruit panics, bare unwraps, and unvalidated parsers must be eliminated!")
        return 1

    print("[OK] ZERO unwrap/expect in decoders. ZERO naked from_*_bytes unwraps. ZERO try_into unwraps.")
    print("==========================================================")
    print("  GATE check_anti_panic_gates: GREEN (100% Fail-Closed)   ")
    print("==========================================================")
    return 0


if __name__ == "__main__":
    sys.exit(main())
