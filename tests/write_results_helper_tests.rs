use teamy_mft::windows::win_rapid_reader::PhysicalReadRequest;
use teamy_mft::windows::win_rapid_reader::PhysicalReadResultEntry;
use teamy_mft::windows::win_rapid_reader::PhysicalReadResults;
use uom::si::information::byte;
use uom::si::u64::Information;

fn info(b: u64) -> Information {
    Information::new::<byte>(b)
}

#[test]
fn writes_blocks_and_preserves_gap_zero() {
    let temp = tempfile::NamedTempFile::new().expect("tmp");
    let path = temp.path().to_path_buf();
    // Two blocks with a gap in between
    let entries = vec![
        PhysicalReadResultEntry {
            request: PhysicalReadRequest {
                physical_offset: info(0),
                logical_offset: info(0),
                length: info(4),
            },
            data: b"ABCD".to_vec(),
        },
        PhysicalReadResultEntry {
            request: PhysicalReadRequest {
                physical_offset: info(1000),
                logical_offset: info(10),
                length: info(3),
            },
            data: b"XYZ".to_vec(),
        },
    ];
    PhysicalReadResults { entries }
        .write_to_file(&path, 20)
        .expect("write ok");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), 20);
    assert_eq!(&bytes[0..4], b"ABCD");
    assert_eq!(&bytes[4..10], &[0u8; 6]); // gap zeroed
    assert_eq!(&bytes[10..13], b"XYZ");
    assert!(bytes[13..].iter().all(|b| *b == 0));
}
