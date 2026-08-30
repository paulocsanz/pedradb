# SAFETY — `pedradb-io-uring`

Production Linux Env (POSIX fallback elsewhere). Public API (`IoUringEnv`,
`open`) is safe. `unsafe` lives
in `src/ring.rs` only (`submit_sqe`). CQE policy is safe Rust in
`src/cqe_kernel.rs`. `posix_fadvise` is **not** here — `pedradb-posix`.

## `submit_sqe` (`ring.rs`)

`io_uring::squeue::SubmissionQueue::push` is `unsafe`. Callers:

- [`UringState::pwrite`] — `buf: &[u8]` and `&File` borrowed until return
- [`UringState::fsync`] — no user buffer; `&File` borrowed until return

Held at every call:

1. Buffer pointer in the SQE (writes) is the live `buf` argument.
2. `Fd` is `file.as_raw_fd()` of that same `File`, open for the wait.
3. `&mut UringState` is exclusive (env `Mutex`). One in-flight SQE.
4. `user_data` is unique (`cqe_kernel::next_user_data`). A leftover CQE
   from a failed `submit_and_wait` cannot match a later op (U1 / F203).
5. Matching CQE is harvested before return, including when submit returns
   `Err` (EINTR after the kernel accepted the SQE). If the CQE is not yet
   visible, **wait again** (F208). Returning Err here would drop the
   caller's `pwrite` buffer while the kernel may still DMA into it —
   unique tags do not keep `buf` alive. A dead ring fd can hang the op
   (preferred to UAF). `ReturnSubmitErr` is the as-is arm only.

The `io-uring` 0.7 crate is inherited TCB (its own `unsafe`). `cargo deny`
advisories cover RUSTSEC. Miri does not emulate a real ring
(`docs/synthetic-field-residuals.md` P2.5).

RFC-0074 P2.1: `cqe_res_ok` has a Verus twin (`verus/cqe_res.rs`); freeze of
twin files, not a ring proof. P2.2: `cqe_ring_model_admitted` is always
false (no `verus/ring_model.rs`). `submit_sqe` stays TCB.

## What this crate must not grow

- SQPOLL / IOPOLL / registered buffers without a new audit.
- Constant per-opcode `user_data` (the U1 hole).
- `posix_fadvise` / `fdatasync` FFI (belongs in `pedradb-posix`).
