use crate::mft::mft_record::MftRecord;
use crate::mft::mft_record_attribute_run_list::MftRecordAttributeRunListOwned;
use crate::mft::mft_record_number::MftRecordNumber;
use crate::ntfs::ntfs_boot_sector::NtfsBootSector;
use crate::ntfs::ntfs_drive_handle::NtfsDriveHandle;
use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::physical_read_results::PhysicalReadResults;
use eyre::WrapErr;
use teamy_windows::handle::get_read_only_drive_handle;
use teamy_windows::string::EasyPCWSTR;
use uom::si::information::byte;
use uom::si::information::mebibyte;
use uom::si::u64::Information;

/// Read the complete MFT using IOCP overlapped reads.
/// drive_letter: 'C', 'D', ...
/// output_path: file path to write final MFT blob
pub fn read_physical_mft(drive_letter: char) -> eyre::Result<PhysicalReadResults> {
    let drive_letter = drive_letter.to_ascii_uppercase();
    let volume_path = format!(r"\\.\{drive_letter}:");
    let volume_path = volume_path
        .easy_pcwstr()
        .wrap_err("Failed to convert volume path to PCWSTR")?;

    {
        // Open blocking handle for boot sector & MFT record parsing
        let drive_handle: NtfsDriveHandle = get_read_only_drive_handle(drive_letter)
            .wrap_err_with(|| format!("Failed to open handle to drive {drive_letter}"))?
            .try_into()
            .wrap_err_with(|| {
                format!(
                    "Failed to convert drive handle for drive {drive_letter} to NtfsDriveHandle"
                )
            })?;

        let boot_sector = NtfsBootSector::try_from_handle(&drive_handle)?;
        let dollar_mft_record = MftRecord::try_from_handle(
            &drive_handle,
            boot_sector.mft_location() + MftRecordNumber::DOLLAR_MFT,
        )?;

        // Gather all non-resident $DATA runlists (could be multiple segments if attribute list used).
        let decoded_runs = MftRecordAttributeRunListOwned::from_mft_record(&dollar_mft_record);
        if decoded_runs.is_empty() {
            eyre::bail!("No non-resident $DATA runs found in $MFT record");
        }
        drop(drive_handle);

        // Build sparse-aware logical plan
        let logical_plan: LogicalReadPlan = decoded_runs
            .into_logical_read_plan(Information::new::<byte>(boot_sector.bytes_per_cluster()));
        if logical_plan.segments.is_empty() {
            eyre::bail!("Logical plan empty (no runs)");
        }

        // Derive physical read plan, merge, chunk and execute with 1 MiB (binary) chunk size (1,048,576 = 1024*1024) for sector alignment
        let chunk_size = Information::new::<mebibyte>(1);
        let mut plan = logical_plan.into_physical_plan();
        plan.align_512().merge_contiguous_reads();
        let plan = plan.chunked(chunk_size);
        let physical_read_results: PhysicalReadResults = plan.read(&volume_path)?;
        Ok(physical_read_results)
    }
}

#[cfg(test)]
mod test {
    use uom::si::information::byte;
    use uom::si::information::mebibyte;
    use uom::si::u64::Information;

    #[test]
    fn it_works() -> eyre::Result<()> {
        assert_eq!(
            Information::new::<byte>(1_048_576).get::<byte>(),
            Information::new::<mebibyte>(1).get::<byte>()
        );

        Ok(())
    }
}
