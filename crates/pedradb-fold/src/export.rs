//! Export / import with verify-by-reopen (RFC-0024 P2.1).

use crate::{FoldCursor, FoldError, FoldRole, FoldStore, PedraFold, Result};
use pedradb_core::StdEnv;
use std::path::Path;

/// Checkpoint the fold directory and require reopened cursor == live cursor.
///
/// # Errors
/// Checkpoint / reopen / cursor mismatch.
pub fn export_fold(live: &mut PedraFold<StdEnv>, dest: &Path) -> Result<FoldCursor> {
    let want = live.cursor();
    live.db_mut().create_checkpoint(dest)?;
    let (got, imported) = PedraFold::open_role(dest, live.role())?;
    drop(imported);
    if got != want {
        return Err(FoldError::Msg(format!(
            "export cursor mismatch live={} dest={}",
            want.seq(),
            got.seq()
        )));
    }
    Ok(got)
}

/// Open a previously exported fold. Recovers the embedded cursor.
///
/// # Errors
/// Pedra open.
pub fn import_fold(path: &Path, role: FoldRole) -> Result<(FoldCursor, PedraFold)> {
    PedraFold::open_role(path, role)
}
