use eyre::bail;
use std::ops::Deref;
use uom::si::information::byte;
use uom::si::usize::Information;

/// Typed size for a single MFT record (entry), in bytes.
///
/// NTFS commonly uses 1024-byte records but can use other sizes (e.g., 4096).
/// Using a dedicated type avoids ambiguous raw `usize` parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MftRecordSize(Information);

impl MftRecordSize {
    pub const MIN_HEADER_SIZE_BYTES: usize = 0x18;

    /// # Errors
    /// Returns an error if the size is smaller than the minimum FILE record header size.
    pub fn new(size: Information) -> eyre::Result<Self> {
        let bytes = size.get::<byte>();
        if bytes < Self::MIN_HEADER_SIZE_BYTES {
            bail!(
                "MFT record size too small: expected at least {} bytes, got {}",
                Self::MIN_HEADER_SIZE_BYTES,
                bytes
            );
        }
        Ok(Self(size))
    }
}

impl Deref for MftRecordSize {
    type Target = Information;
    fn deref(&self) -> &Information {
        &self.0
    }
}

impl From<MftRecordSize> for Information {
    fn from(val: MftRecordSize) -> Self {
        val.0
    }
}
