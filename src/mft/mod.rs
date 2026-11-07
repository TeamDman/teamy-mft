pub mod fast_entry;
pub mod fast_fixup;
pub mod mft_convert_to_path_collection;
pub mod mft_file;
pub mod mft_location;
pub mod mft_record;
pub mod mft_record_attribute;
pub mod mft_record_attribute_iter;
pub mod mft_record_attribute_non_resident_header;
pub mod mft_record_attribute_run_list;
pub mod mft_record_attribute_x80_data_attribute;
pub mod mft_record_flags;
pub mod mft_record_index;
pub mod mft_record_iter;
pub mod mft_record_location;
pub mod mft_record_number;
pub mod mft_record_reference;
pub mod mft_record_size;
pub mod mft_sequence_number;
pub mod path_resolve;
mod physical_mft_read_request;

pub use physical_mft_read_request::PhysicalMftReadRequest;

/// Bounded, on-disk-form test records with USA fixups, a directory, hard links,
/// and a deleted file. No live filesystem metadata is used.
#[cfg(test)]
pub(crate) fn sample_mft_records() -> Vec<u8> {
    let mut records = vec![0; 9 * 1024];
    for (index, record) in records.chunks_mut(1024).enumerate() {
        record[..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&48u16.to_le_bytes());
        record[6..8].copy_from_slice(&3u16.to_le_bytes());
        record[16..18].copy_from_slice(&1u16.to_le_bytes());
        record[20..22].copy_from_slice(&56u16.to_le_bytes());
        record[28..32].copy_from_slice(&1024u32.to_le_bytes());
        record[44..48].copy_from_slice(&u32::try_from(index).unwrap().to_le_bytes());
        let (flags, names): (u16, &[(u64, &str)]) = match index {
            5 => (3, &[]),
            6 => (3, &[(5, "folder")]),
            7 => (1, &[(6, "example.txt"), (5, "linked.txt")]),
            8 => (0, &[(6, "gone.txt")]),
            _ => (0, &[]),
        };
        record[22..24].copy_from_slice(&flags.to_le_bytes());
        record[18..20].copy_from_slice(&u16::try_from(names.len()).unwrap().to_le_bytes());
        let mut offset = 56;
        for (id, &(parent, name)) in names.iter().enumerate() {
            let units = name.encode_utf16().collect::<Vec<_>>();
            let value_size = 66 + units.len() * 2;
            let attr_size = (24 + value_size + 7) & !7;
            record[offset..offset + 4].copy_from_slice(&0x30u32.to_le_bytes());
            record[offset + 4..offset + 8]
                .copy_from_slice(&u32::try_from(attr_size).unwrap().to_le_bytes());
            record[offset + 14..offset + 16]
                .copy_from_slice(&u16::try_from(id).unwrap().to_le_bytes());
            record[offset + 16..offset + 20]
                .copy_from_slice(&u32::try_from(value_size).unwrap().to_le_bytes());
            record[offset + 20..offset + 22].copy_from_slice(&24u16.to_le_bytes());
            let value = offset + 24;
            record[value..value + 8].copy_from_slice(&((1u64 << 48) | parent).to_le_bytes());
            record[value + 64] = u8::try_from(units.len()).unwrap();
            record[value + 65] = 1;
            for (i, unit) in units.into_iter().enumerate() {
                record[value + 66 + 2 * i..value + 68 + 2 * i].copy_from_slice(&unit.to_le_bytes());
            }
            offset += attr_size;
        }
        record[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        record[24..28].copy_from_slice(&u32::try_from(offset + 4).unwrap().to_le_bytes());
        record[48..50].copy_from_slice(&0xa55au16.to_le_bytes());
        for (sector, tail) in [510, 1022].into_iter().enumerate() {
            let saved = [record[tail], record[tail + 1]];
            record[50 + sector * 2..52 + sector * 2].copy_from_slice(&saved);
            record[tail..tail + 2].copy_from_slice(&0xa55au16.to_le_bytes());
        }
    }
    records
}
