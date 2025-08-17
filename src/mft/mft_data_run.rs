use crate::mft::mft_record::MftRecord;
use eyre::eyre;
/// Data run information
#[derive(Debug)]
pub struct DataRun {
    pub length: u64,  // Length in clusters
    pub cluster: i64, // Cluster offset (can be negative for relative positioning)
}

/// Parses an MFT record to extract data runs from the DATA attribute (0x80)
pub fn parse_mft_record_for_data_attribute(
    record: &MftRecord,
) -> eyre::Result<Vec<DataRun>> {
    let record = record.data;
    // Get the offset to the first attribute (typically at offset 20)
    let attr_offset = u16::from_le_bytes([record[20], record[21]]) as usize;
    let mut read_ptr = attr_offset;

    while read_ptr < record.len() {
        // Read attribute header
        if read_ptr + 8 > record.len() {
            break;
        }

        let attr_type = u32::from_le_bytes([
            record[read_ptr],
            record[read_ptr + 1],
            record[read_ptr + 2],
            record[read_ptr + 3],
        ]);

        // Check for end marker
        if attr_type == 0xffffffff {
            break;
        }

        let attr_length = u32::from_le_bytes([
            record[read_ptr + 4],
            record[read_ptr + 5],
            record[read_ptr + 6],
            record[read_ptr + 7],
        ]) as usize;

        if attr_length == 0 {
            break;
        }

        // Check if this is the DATA attribute (0x80)
        if attr_type == 0x80 {
            // Check if it's non-resident (byte at offset 8 should be != 0)
            if read_ptr + 8 < record.len() && record[read_ptr + 8] != 0 {
                // Get the data runs offset (at offset 32 from attribute start)
                if read_ptr + 34 <= record.len() {
                    let run_offset =
                        u16::from_le_bytes([record[read_ptr + 32], record[read_ptr + 33]]) as usize;

                    let data_runs_start = read_ptr + run_offset;
                    let data_runs_end = read_ptr + attr_length;

                    if data_runs_start < data_runs_end && data_runs_end <= record.len() {
                        return decode_data_runs(&record[data_runs_start..data_runs_end]);
                    }
                }
            }
        }

        read_ptr += attr_length;
    }

    Err(eyre!("Could not find DATA attribute (0x80) in MFT record"))
}

/// Decodes NTFS data runs
fn decode_data_runs(data_runs: &[u8]) -> eyre::Result<Vec<DataRun>> {
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
