use crate::read::physical_read_request::PhysicalReadRequest;
use uom::si::information::byte;
use uom::si::u64::Information;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PhysicalReadResultEntry {
    pub request: PhysicalReadRequest,
    pub data: Vec<u8>,
}

impl Ord for PhysicalReadResultEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.request.cmp(&other.request)
    }
}
impl PartialOrd for PhysicalReadResultEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// --------------- Convenience Helpers ---------------
impl PhysicalReadResultEntry {
    /// Logical start offset in bytes as u64
    pub fn logical_offset_bytes(&self) -> u64 {
        self.request.logical_offset.get::<byte>()
    }
}

#[derive(Debug)]
pub struct PhysicalReadResults {
    pub entries: Vec<PhysicalReadResultEntry>,
    pub total_size: Information,
}
impl PhysicalReadResults {
    /// Consumes the results and writes them to a file (pre-sizing & zero-filling gaps by allocation).
    pub fn write_to_file(self, output_path: impl AsRef<std::path::Path>) -> eyre::Result<()> {
        use std::io::Seek;
        use std::io::SeekFrom;
        use std::io::Write;
        let mut entries = self.entries;
        if entries.is_empty() {
            let file = std::fs::File::create(output_path)?;
            file.set_len(self.total_size.get::<byte>())?;
            return Ok(());
        }

        let file = std::fs::File::create(output_path)?;
        file.set_len(self.total_size.get::<byte>())?;

        let mut writer = std::io::BufWriter::new(file);

        entries.sort();
        for e in entries {
            // If we over-aligned earlier, we may have leading bytes before logical_offset.
            // Compute how many leading bytes to skip: logical - physical delta (clamped).
            let phys = e.request.physical_offset.get::<byte>();
            let log = e.request.logical_offset.get::<byte>();
            let delta = log.saturating_sub(phys); // bytes to skip in buffer
            if delta as usize > e.data.len() {
                continue;
            }
            let slice = &e.data[delta as usize..];
            // We must not exceed the intended logical length.
            let intended = e.request.length.get::<byte>() - delta; // inflated by alignment
            let logical_len = (e.request.logical_end().get::<byte>() - log).min(intended);
            let max_len = logical_len as usize;
            let used_len = std::cmp::min(max_len, slice.len());
            let slice = &slice[..used_len];
            writer.seek(SeekFrom::Start(log))?;
            writer.write_all(slice)?;
        }
        writer.flush()?;
        Ok(())
    }
}
