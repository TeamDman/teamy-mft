use crate::mft::fast_fixup::FixupStats;
use crate::mft::fast_fixup::apply_fixups_parallel;
use bytes::Bytes;
use eyre::Context;
use std::fmt::Debug;
use std::io::Read;
use std::ops::Deref;
use std::path::Path;
use std::time::Instant;
use thousands::Separable;
use tracing::debug;
use tracing::instrument;
use uom::si::f64::Information;
use uom::si::information::byte;

pub struct MftFile {
    bytes: Bytes,
}
impl Debug for MftFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MftFile")
            .field("size", &self.size().get_human())
            .field("entry_size", &self.entry_size().get_human())
            .field("entry_count", &self.entry_count().separate_with_commas())
            .finish()
    }
}
impl Deref for MftFile {
    type Target = Bytes;
    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}
impl MftFile {
    pub fn size(&self) -> Information {
        Information::new::<byte>(self.bytes.len() as f64)
    }
    pub fn entry_size(&self) -> Information {
        if self.len() < 0x20 {
            return Information::new::<byte>(1024.0);
        }
        let size = u32::from_le_bytes([self[0x1C], self[0x1D], self[0x1E], self[0x1F]]);
        if size == 0 {
            Information::new::<byte>(1024.0)
        } else {
            Information::new::<byte>(size as f64)
        }
    }
    pub fn entry_count(&self) -> usize {
        let entry_size_bytes = self.entry_size().get::<byte>() as usize;
        if entry_size_bytes == 0 {
            0
        } else {
            self.bytes.len() / entry_size_bytes
        }
    }

    #[instrument(level = "debug")]
    pub fn from_path(mft_file_path: &Path) -> eyre::Result<Self> {
        // Open file
        let file = std::fs::File::open(mft_file_path)
            .wrap_err_with(|| format!("Failed to open {}", mft_file_path.display()))?;

        debug!("Opened MFT file: {}", mft_file_path.display());

        // Determine file size
        let file_size_bytes = file
            .metadata()
            .wrap_err_with(|| format!("Failed to get metadata for {}", mft_file_path.display()))?
            .len() as usize;
        let mft_file_size = Information::new::<byte>(file_size_bytes as f64);
        if file_size_bytes < 1024 {
            eyre::bail!("MFT file too small: {}", mft_file_path.display());
        }

        // Read all bytes
        debug!("Reading cached bytes: {}", mft_file_size.get_human());
        let read_start = Instant::now();
        let bytes = {
            let mut buf = Vec::with_capacity(file_size_bytes);
            let mut reader = std::io::BufReader::new(&file);
            reader
                .read_to_end(&mut buf)
                .wrap_err_with(|| format!("Failed to read {}", mft_file_path.display()))?;
            buf
        };

        // Defer fixups and struct construction to from_bytes
        let rtn = MftFile::from_bytes(bytes)?;

        // Log summary
        debug!(
            "Read {} in {:.2?}, found entry size {} and {} entries",
            mft_file_size.get_human(),
            read_start.elapsed(),
            rtn.entry_size().get_human(),
            rtn.entry_count().separate_with_commas()
        );

        Ok(rtn)
    }

    /// Construct from in-memory bytes that need fixups; applies fixups and stores Bytes.
    #[instrument(level = "debug", skip_all)]
    pub fn from_bytes(mut raw: Vec<u8>) -> eyre::Result<Self> {
        // Ensure we have enough bytes to read the entry size field at 0x1C..=0x1F
        if raw.len() < 0x20 {
            eyre::bail!(
                "MFT buffer too small ({} bytes); need at least 0x20 to read entry size",
                raw.len()
            );
        }

        // Read entry size in bytes (little-endian u32 at offset 0x1C)
        let entry_size_bytes =
            u32::from_le_bytes([raw[0x1C], raw[0x1D], raw[0x1E], raw[0x1F]]) as usize;

        // Validate the entry size field
        if entry_size_bytes == 0 {
            eyre::bail!("Entry size field is zero (invalid/unknown)");
        }

        // Ensure the buffer length aligns to the entry size
        if !raw.len().is_multiple_of(entry_size_bytes) {
            eyre::bail!(
                "Buffer length ({}) is not a multiple of entry size ({})",
                raw.len(),
                entry_size_bytes
            );
        }
        let _stats: FixupStats = apply_fixups_parallel(&mut raw, entry_size_bytes);
        Ok(MftFile {
            bytes: Bytes::from(raw),
        })
    }
}
