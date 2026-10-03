//! WAL ship mapped to fold cursor expiry (RFC-0024 P2.2).

use crate::{FoldCursor, FoldError, Result};
use pedradb_replicate::{ShipError, WalShipper};

/// Pull WAL bytes; rotate/truncate becomes [`FoldError::CursorExpired`].
///
/// # Errors
/// I/O or cursor expired.
pub fn ship_pull(shipper: &mut WalShipper) -> Result<Option<Vec<u8>>> {
    match shipper.pull() {
        Ok(v) => Ok(v),
        Err(ShipError::WalRotated { file_len, cursor }) => Err(FoldError::CursorExpired {
            pin: cursor,
            first_retained: file_len,
        }),
        Err(e) => Err(FoldError::Ship(e)),
    }
}

/// Helper that remembers the last good ship offset as a fold cursor analog.
pub struct FoldShip {
    shipper: WalShipper,
}

impl FoldShip {
    /// Follow the primary WAL from the current end.
    ///
    /// # Errors
    /// Stat WAL.
    pub fn follow(primary_dir: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self {
            shipper: WalShipper::follow(primary_dir)?,
        })
    }

    /// From byte 0.
    #[must_use]
    pub fn from_start(primary_dir: impl AsRef<std::path::Path>) -> Self {
        Self {
            shipper: WalShipper::from_start(primary_dir),
        }
    }

    /// Pull or expire.
    ///
    /// # Errors
    /// See [`ship_pull`].
    pub fn pull(&mut self) -> Result<Option<Vec<u8>>> {
        ship_pull(&mut self.shipper)
    }

    /// Byte cursor (not a seq pin).
    #[must_use]
    pub fn offset(&self) -> FoldCursor {
        FoldCursor(self.shipper.offset())
    }
}
