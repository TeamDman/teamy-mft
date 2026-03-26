use crate::mft::fast_fixup::apply_fixups_parallel;
use crate::mft::mft_record_iter::MftRecordIter;
use crate::mft::mft_record_size::MftRecordSize;
use bytes::Bytes;
use bytes::BytesMut;
use eyre::Context;
use eyre::bail;
use humansize::BINARY;
use std::fmt::Debug;
use std::io::Read;
use std::ops::Deref;
use std::path::Path;
use std::time::Instant;
use teamy_uom_extensions::HumanInformationExt;
use thousands::Separable;
use tracing::debug;
use tracing::debug_span;
use tracing::instrument;
use uom::si::information::byte;
use uom::si::usize::Information;

pub struct MftFile {
    bytes: Bytes,
}
impl Debug for MftFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MftFile")
            .field("size", &self.size().format_human(BINARY))
            .field("entry_size", &self.record_size().format_human(BINARY))
            .field("entry_count", &self.record_count().separate_with_commas())
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
    // mftf[impl cached-stream.record-size-field]
    // mftf[impl cached-stream.fixed-record-size]
    // mftf[impl cached-stream.fixups-applied-before-iteration]
    fn validate_and_apply_fixups(raw: &mut [u8]) -> eyre::Result<()> {
        {
            let _span = debug_span!("validate_minimum_header_size", raw_len = raw.len()).entered();
            if raw.len() < 0x20 {
                bail!(
                    "MFT buffer too small ({} bytes); need at least 0x20 to read entry size",
                    raw.len()
                );
            }
        }

        let entry_size_bytes = {
            let _span = debug_span!("read_entry_size_from_header").entered();
            u32::from_le_bytes([raw[0x1C], raw[0x1D], raw[0x1E], raw[0x1F]]) as usize
        };

        {
            let _span = debug_span!(
                "validate_entry_size",
                entry_size_bytes = entry_size_bytes,
                raw_len = raw.len()
            )
            .entered();
            if entry_size_bytes == 0 {
                bail!("Entry size field is zero (invalid/unknown)");
            }
            if !raw.len().is_multiple_of(entry_size_bytes) {
                bail!(
                    "Buffer length ({}) is not a multiple of entry size ({})",
                    raw.len(),
                    entry_size_bytes
                );
            }
        }

        {
            let _span =
                debug_span!("apply_fixups_parallel", entry_size_bytes = entry_size_bytes).entered();
            let _stats = apply_fixups_parallel(raw, entry_size_bytes);
        }

        Ok(())
    }

    pub fn size(&self) -> Information {
        Information::new::<byte>(self.bytes.len())
    }
    /// # Panics
    ///
    /// Panics if the MFT entry size field is less than 0 or if the buffer is too small to read the entry size field.
    pub fn record_size(&self) -> MftRecordSize {
        debug_assert!(
            self.len() >= 0x20,
            "MFT buffer too small to read entry size field, got {} bytes",
            self.len()
        );
        let size = u32::from_le_bytes([self[0x1C], self[0x1D], self[0x1E], self[0x1F]]) as usize;
        debug_assert!(size > 0, "MFT entry size field is zero (invalid/unknown)");
        MftRecordSize::new(Information::new::<byte>(size)).expect("MFT record size must be valid")
    }

    /// # Panics
    ///
    /// Panics if the MFT entry size field is zero or if the buffer is too small to read the entry size field.
    pub fn record_count(&self) -> usize {
        let entry_size_bytes = self.record_size().get::<byte>();
        debug_assert!(
            entry_size_bytes > 0,
            "MFT entry size field is zero (invalid/unknown)"
        );
        self.bytes.len() / entry_size_bytes
    }

    /// Load an MFT file from the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened, read, or parsed.
    #[instrument(level = "debug")]
    pub fn from_path(mft_file_path: &Path) -> eyre::Result<Self> {
        let file = {
            let _span = debug_span!("open_file", path = %mft_file_path.display()).entered();
            std::fs::File::open(mft_file_path)
                .wrap_err_with(|| format!("Failed to open {}", mft_file_path.display()))?
        };

        debug!("Opened MFT file: {}", mft_file_path.display());

        let mft_file_size = {
            let _span = debug_span!("read_metadata", path = %mft_file_path.display()).entered();
            Information::new::<byte>(
                usize::try_from(
                    file.metadata()
                        .wrap_err_with(|| {
                            format!("Failed to get metadata for {}", mft_file_path.display())
                        })?
                        .len(),
                )
                .wrap_err("File size too large for usize")?,
            )
        };
        if mft_file_size < Information::new::<byte>(1024) {
            bail!("MFT file too small: {}", mft_file_path.display());
        }

        // Read all bytes
        debug!(
            "Reading cached bytes: {}",
            mft_file_size.format_human(BINARY)
        );
        let read_start = Instant::now();
        let bytes = {
            let _span = debug_span!(
                "read_all_bytes",
                path = %mft_file_path.display(),
                file_size_bytes = mft_file_size.get::<byte>()
            )
            .entered();
            let mut buf = Vec::with_capacity(mft_file_size.get::<byte>());
            let mut reader = std::io::BufReader::new(&file);
            reader
                .read_to_end(&mut buf)
                .wrap_err_with(|| format!("Failed to read {}", mft_file_path.display()))?;
            BytesMut::from(Bytes::from(buf))
        };

        let rtn = {
            let _span = debug_span!("construct_from_bytes").entered();
            MftFile::from_bytes(bytes)?
        };

        // Log summary
        debug!(
            "Read {} in {:.2?}, found entry size {} bytes and {} entries",
            mft_file_size.format_human(BINARY),
            read_start.elapsed(),
            rtn.record_size().get::<byte>().separate_with_commas(),
            rtn.record_count().separate_with_commas()
        );

        Ok(rtn)
    }

    /// Construct from in-memory bytes that need fixups; applies fixups and stores Bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are invalid or fixups fail.
    #[instrument(level = "debug", skip_all)]
    pub fn from_bytes(mut raw: BytesMut) -> eyre::Result<Self> {
        Self::validate_and_apply_fixups(raw.as_mut())?;

        let bytes = {
            let _span = debug_span!("freeze_bytes").entered();
            raw.freeze()
        };

        Ok(MftFile { bytes })
    }

    /// Construct from owned logical MFT bytes that still need fixups applied.
    ///
    /// This is useful when reconstructing a contiguous in-memory MFT from a
    /// discontiguous physical read, avoiding a write-to-disk then read-back cycle.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are invalid or fixups fail.
    #[instrument(level = "debug", skip_all)]
    pub fn from_vec(mut raw: Vec<u8>) -> eyre::Result<Self> {
        Self::validate_and_apply_fixups(&mut raw)?;
        Ok(MftFile {
            bytes: Bytes::from(raw),
        })
    }

    /// Iterate over fixed-size records contained in this MFT file.
    ///
    /// The logical MFT stream starts directly with record 0 (`FILE`), so there is
    /// no file-level header to skip before iteration begins.
    ///
    /// This creates zero-copy `MftRecord` instances by slicing the shared
    /// `Bytes` buffer. No signature validation is performed.
    /// The caller is responsible for ensuring fixups were already applied
    /// (handled by `MftFile::from_bytes`/`from_path`).
    #[inline]
    // mftf[impl cached-stream.begins-at-record-zero]
    // mftf[impl record-iteration.contiguous-fixed-size-slices]
    pub fn iter_records(&self) -> MftRecordIter {
        MftRecordIter::new(self.bytes.clone(), self.record_size())
    }
}
