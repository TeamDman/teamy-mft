use eyre::Context;
use mft::fast_fixup::apply_fixups_parallel;
use std::fmt::Debug;
use std::io::Read;
use std::ops::Deref;
use std::path::Path;
use std::time::Instant;
use thousands::Separable;
use tracing::debug;
use tracing::instrument;
use uom::si::f64::Information;
use uom::si::f64::Time;
use uom::si::frequency::hertz;
use uom::si::information::byte;
use uom::si::time::second;

pub struct MftFile {
    pub bytes: Vec<u8>,
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
    type Target = [u8];
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
    pub fn read(mft_file_path: &Path) -> eyre::Result<Self> {
        let file = std::fs::File::open(mft_file_path)
            .wrap_err_with(|| format!("Failed to open {}", mft_file_path.display()))?;

        debug!("Opened MFT file: {}", mft_file_path.display());

        // file size
        let file_size_bytes = file
            .metadata()
            .wrap_err_with(|| format!("Failed to get metadata for {}", mft_file_path.display()))?
            .len() as usize;
        let mft_file_size = Information::new::<byte>(file_size_bytes as f64);
        if file_size_bytes < 1024 {
            eyre::bail!("MFT file too small: {}", mft_file_path.display());
        }

        // read
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
        
        let rtn = MftFile { bytes };

        debug!(
            "Read {} in {:.2?}, found entry size {} and {} entries",
            mft_file_size.get_human(),
            read_start.elapsed(),
            rtn.entry_size().get_human(),
            rtn.entry_count().separate_with_commas()
        );


        Ok(rtn)
    }

    #[instrument(level = "debug")]
    pub fn apply_fixups_in_place(&mut self) -> eyre::Result<()> {
        let entry_size_bytes = self.entry_size().get::<byte>() as usize;
        if entry_size_bytes == 0 || !self.bytes.len().is_multiple_of(entry_size_bytes) {
            eyre::bail!("Unaligned entry size");
        }
        let entry_count = self.bytes.len() / entry_size_bytes;
        debug!(
            "Detected entry size: {} bytes, total entries: {}",
            entry_size_bytes, entry_count
        );

        let fixup_start = Instant::now();
        let fixup_stats = apply_fixups_parallel(&mut self.bytes, entry_size_bytes);
        let fixup_elapsed = Time::new::<second>(fixup_start.elapsed().as_secs_f64());
        let fixup_rate = self.size() / fixup_elapsed;
        debug!(
            "Took {} ({}/s) applied/already/invalid={}/{}/{}",
            fixup_elapsed.get_human(),
            fixup_rate.get::<hertz>().trunc().separate_with_commas(),
            fixup_stats.applied.separate_with_commas(),
            fixup_stats.already_applied.separate_with_commas(),
            fixup_stats.invalid.separate_with_commas()
        );
        Ok(())
    }
}
