use teamy_mft::windows::win_rapid_reader::PhysicalReadPlan;
use uom::si::information::byte;
use uom::si::u64::Information;

fn info(bytes: u64) -> Information {
    Information::new::<byte>(bytes)
}

#[test]
fn merge_adjacent_pushes() {
    let mut r = PhysicalReadPlan::new();
    r.push(info(0), info(0), info(100));
    r.push(info(100), info(100), info(50)); // contiguous -> should merge after merge_contiguous_reads
    r.merge_contiguous_reads();
    assert_eq!(r.num_requests(), 1, "Expected contiguous pushes to merge");
    assert_eq!(r.requests()[0].physical_offset.get::<byte>(), 0);
    assert_eq!(r.requests()[0].logical_offset.get::<byte>(), 0);
    assert_eq!(r.requests()[0].length.get::<byte>(), 150);
    assert_eq!(r.total_requested_information().get::<byte>(), 150);
}

#[test]
fn non_adjacent_does_not_merge() {
    let mut r = PhysicalReadPlan::new();
    r.push(info(0), info(0), info(100));
    r.push(info(101), info(101), info(50)); // gap of 1
    r.merge_contiguous_reads();
    assert_eq!(r.num_requests(), 2, "Non-contiguous pushes must not merge");
}

#[test]
fn chunking_splits_without_merging_chunks() {
    let mut r = PhysicalReadPlan::new();
    r.push(info(0), info(0), info(300)); // single extent
    let chunked = r.chunked(info(128));
    // 300 bytes in 128-byte chunks => 128,128,44 (3 chunks)
    assert_eq!(
        chunked.num_requests(),
        3,
        "Chunking should split into 3 parts"
    );
    let reqs = chunked.requests();
    assert_eq!(reqs[0].physical_offset.get::<byte>(), 0);
    assert_eq!(reqs[0].length.get::<byte>(), 128);
    assert_eq!(reqs[1].physical_offset.get::<byte>(), 128);
    assert_eq!(reqs[1].length.get::<byte>(), 128);
    assert_eq!(reqs[2].physical_offset.get::<byte>(), 256);
    assert_eq!(reqs[2].length.get::<byte>(), 44);
    assert_eq!(
        chunked.total_requested_information().get::<byte>(),
        300,
        "Total requested should remain constant"
    );
}

#[test]
fn chunking_respects_exact_division() {
    let mut r = PhysicalReadPlan::new();
    r.push(info(4096), info(4096), info(4096));
    let c = r.chunked(info(1024));
    assert_eq!(c.num_requests(), 4);
    for (i, req) in c.requests().iter().enumerate() {
        assert_eq!(req.physical_offset.get::<byte>(), 4096 + (i as u64) * 1024);
        assert_eq!(req.logical_offset.get::<byte>(), 4096 + (i as u64) * 1024);
        assert_eq!(req.length.get::<byte>(), 1024);
    }
}

#[test]
fn zero_length_push_ignored() {
    let mut r = PhysicalReadPlan::new();
    r.push(info(0), info(0), info(0));
    assert!(r.is_empty());
}
