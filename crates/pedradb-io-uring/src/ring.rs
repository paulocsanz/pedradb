//! Linux io_uring submit + harvest.
//!
//! Safe methods on [`UringState`] borrow `File` / `[u8]` for the wait.
//! The only `unsafe` is [`submit_sqe`] (SQE push). Policy: `cqe_kernel`
//! (unique `user_data`, harvest on submit Err). Invariants: crate `SAFETY.md`.

use std::fs::File;
use std::io;
use std::os::unix::io::AsRawFd;

use super::cqe_kernel::{
    cqe_act, cqe_res_ok, next_user_data, submit_complete_act, CqeAct, SubmitCompleteAct,
    FIRST_USER_DATA,
};

/// Ring + monotonic SQE tags (U1: never reuse a tag while a leftover CQE
/// from a failed submit could still sit in the CQ).
pub(crate) struct UringState {
    ring: io_uring::IoUring,
    next_tag: u64,
    /// Test-only: replace the next harvested CQE `res` (RFC-0050 P0.2).
    /// Applied *after* `io_uring_enter` so the real buffer stays valid.
    #[cfg(test)]
    inject_cqe: Option<i32>,
}

impl UringState {
    pub(crate) fn new(ring: io_uring::IoUring) -> Self {
        Self {
            ring,
            next_tag: FIRST_USER_DATA,
            #[cfg(test)]
            inject_cqe: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn inject_next_cqe(&mut self, res: i32) {
        self.inject_cqe = Some(res);
    }

    /// `pwrite` at `offset`. Short writes (`res` as `u32` SQE length) are
    /// the `Write` contract, not UB.
    pub(crate) fn pwrite(&mut self, file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let fd = io_uring::types::Fd(file.as_raw_fd());
        let entry = io_uring::opcode::Write::new(fd, buf.as_ptr(), buf.len() as u32)
            .offset(offset)
            .build();
        // SAFETY: `buf` and `file` are borrowed until this returns. `&mut self`
        // is exclusive ring access (caller holds the env mutex).
        let res = unsafe { submit_sqe(self, entry)? };
        if !cqe_res_ok(res) {
            return Err(io::Error::from_raw_os_error(-res));
        }
        Ok(res as usize)
    }

    /// `fsync` / `fdatasync` on `file` (also valid for a directory fd).
    pub(crate) fn fsync(&mut self, file: &File, datasync: bool) -> io::Result<()> {
        let fd = io_uring::types::Fd(file.as_raw_fd());
        let mut op = io_uring::opcode::Fsync::new(fd);
        if datasync {
            op = op.flags(io_uring::types::FsyncFlags::DATASYNC);
        }
        let entry = op.build();
        // SAFETY: no user buffer. `file` is open until harvest returns.
        // `&mut self` is exclusive ring access.
        let res = unsafe { submit_sqe(self, entry)? };
        if !cqe_res_ok(res) {
            return Err(io::Error::from_raw_os_error(-res));
        }
        Ok(())
    }
}

/// Drain currently visible CQEs; take ours, drop leftovers (F203).
fn harvest_ready_cqe(ring: &mut io_uring::IoUring, tag: u64) -> Option<i32> {
    while let Some(cqe) = ring.completion().next() {
        match cqe_act(cqe.user_data(), tag) {
            CqeAct::Take => return Some(cqe.result()),
            CqeAct::Discard => {}
        }
    }
    None
}

/// Push `entry`, wait for its CQE. Unique `user_data` per issue (U1).
///
/// Harvests a matching CQE even when `submit_and_wait` returns Err (EINTR
/// after the kernel accepted the SQE). If the CQE is not there yet, wait
/// again (F208): returning would drop the caller's `pwrite` buffer while
/// the kernel may still DMA into it. Unique tags stop a leftover from
/// matching a *later* op; they do not keep `buf` alive.
///
/// # Safety
///
/// Caller must ensure:
/// - any buffer pointer in `entry` stays valid until this function returns
/// - the fd in `entry` is a live descriptor of an open `File` owned across
///   the call
/// - `state` is the exclusive owner of the ring (mutex held)
unsafe fn submit_sqe(state: &mut UringState, entry: io_uring::squeue::Entry) -> io::Result<i32> {
    let tag = next_user_data(&mut state.next_tag);
    let entry = entry.user_data(tag);
    // SAFETY: obligations listed in the function's `# Safety` are held by
    // [`UringState::pwrite`] / [`UringState::fsync`]. `push` requires
    // exclusive SQ access — `state` is `&mut`. The kernel must not observe
    // `entry`'s buffer after we return: we harvest this `tag` before return
    // (WaitMore on submit Err, F208). `ReturnSubmitErr` is not taken after
    // a successful push.
    unsafe {
        state
            .ring
            .submission()
            .push(&entry)
            .map_err(|e| io::Error::other(format!("io_uring sq full: {e}")))?;
    }
    let mut submit_ok;
    let mut last_err: Option<io::Error>;
    match state.ring.submit_and_wait(1) {
        Ok(_) => {
            submit_ok = true;
            last_err = None;
        }
        Err(e) => {
            submit_ok = false;
            last_err = Some(e);
        }
    }
    loop {
        let harvested = harvest_ready_cqe(&mut state.ring, tag);
        match (
            submit_complete_act(submit_ok, harvested.is_some()),
            harvested,
        ) {
            (SubmitCompleteAct::UseHarvested, Some(res)) => {
                #[cfg(test)]
                let res = state.inject_cqe.take().unwrap_or(res);
                return Ok(res);
            }
            (SubmitCompleteAct::UseHarvested, None) => {
                unreachable!("cqe kernel: UseHarvested implies a CQE");
            }
            (SubmitCompleteAct::ReturnSubmitErr, _) => {
                return Err(last_err
                    .take()
                    .unwrap_or_else(|| io::Error::other("io_uring submit failed without errno")));
            }
            (SubmitCompleteAct::WaitMore, _) => match state.ring.submit_and_wait(1) {
                Ok(_) => {
                    submit_ok = true;
                }
                Err(e) => {
                    submit_ok = false;
                    last_err = Some(e);
                }
            },
        }
    }
}
