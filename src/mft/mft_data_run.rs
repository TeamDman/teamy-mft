use crate::mft::mft_record::MftRecord;
use eyre::Result;
/// Data run information
#[derive(Debug)]
pub struct DataRun {
    pub length: u64,  // Length in clusters
    pub cluster: i64, // Cluster offset (can be negative for relative positioning)
}

/// Parses an MFT record to extract data runs from the DATA attribute (0x80)
pub fn parse_mft_record_for_data_attribute(record: &MftRecord) -> Result<Vec<DataRun>> {
    let runlist = record.get_data_attribute_runlist()?; // underlying attribute helpers validate bounds
    decode_data_runs(runlist)
}

/// Decodes NTFS data runs
fn decode_data_runs(data_runs: &[u8]) -> Result<Vec<DataRun>> {
    let mut runs = Vec::new();
    let mut decode_pos = 0;

    while decode_pos < data_runs.len() {
        let header = data_runs[decode_pos];

        // End of data runs
        if header == 0 {
            break;
        }

        let offset_bytes = (header & 0xf0) >> 4;
        let length_bytes = header & 0x0f;

        if offset_bytes == 0 || length_bytes == 0 {
            break;
        }

        decode_pos += 1;

        // Read length (little-endian)
        if decode_pos + length_bytes as usize > data_runs.len() {
            break;
        }

        let mut length = 0u64;
        for i in 0..length_bytes {
            length |= (data_runs[decode_pos + i as usize] as u64) << (i * 8);
        }
        decode_pos += length_bytes as usize;

        // Read offset (little-endian, signed)
        if decode_pos + offset_bytes as usize > data_runs.len() {
            break;
        }

        let mut cluster = 0i64;
        for i in 0..offset_bytes {
            cluster |= (data_runs[decode_pos + i as usize] as i64) << (i * 8);
        }

        // Handle sign extension for the offset
        if offset_bytes > 0 {
            let sign_bit = 1i64 << (offset_bytes * 8 - 1);
            if cluster & sign_bit != 0 {
                cluster |= !((1i64 << (offset_bytes * 8)) - 1);
            }
        }

        decode_pos += offset_bytes as usize;

        runs.push(DataRun { length, cluster });
    }

    Ok(runs)
}
