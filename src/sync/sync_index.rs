use crate::mft::mft_convert_to_path_collection::convert_mft_file_to_path_collection;
use crate::mft::mft_file::MftFile;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use crate::search_index::search_index_bytes::SearchIndexBytesMut;
use crate::sync::DriveSyncInfo;
use crate::sync::IfExistsOutputBehaviour;
use crate::sync::sync_phase::SyncPhase;
use crate::sync::sync_phase::bytes_human;
use crate::sync::sync_phase::bytes_per_second;
use crate::sync::sync_phase::bytes_per_second_human;
use crate::sync::sync_phase::count_per_second;
use crate::sync::sync_phase::count_per_second_human;
use crate::sync::sync_phase::elapsed_human;
use crate::sync::sync_phase::elapsed_ms;
use crate::sync::sync_phase::u64_from_usize;
use eyre::Context;
use eyre::bail;
use itertools::Itertools;
use tracing::debug;
use tracing::info;
use tracing::info_span;
use uom::si::information::byte;

#[derive(Debug)]
pub struct SyncIndex;

impl SyncIndex {
    /// Validate the sync can proceed before any index writes begin.
    ///
    /// # Errors
    ///
    /// Returns an error if `if_exists` is `Abort` and any index output already exists.
    pub fn invoke_preflight(
        drive_infos: Vec<DriveSyncInfo>,
        if_exists: &IfExistsOutputBehaviour,
    ) -> eyre::Result<Vec<DriveSyncInfo>> {
        let mut rtn = Vec::with_capacity(drive_infos.len());
        for info in drive_infos {
            let index_exists = info.index_output_path.exists();
            match (index_exists, if_exists) {
                (false, _) | (true, IfExistsOutputBehaviour::Overwrite) => {
                    rtn.push(info);
                }
                (true, IfExistsOutputBehaviour::Skip) => {
                    debug!(
                        drive = %info.drive_letter,
                        path = %info.index_output_path.display(),
                        "Skipping existing index output"
                    );
                }
                (true, IfExistsOutputBehaviour::Abort) => {
                    bail!(
                        "Aborting sync: {} already exists",
                        info.index_output_path.display()
                    );
                }
            }
        }
        Ok(rtn)
    }

    /// Build `.mft_search_index` files from cached MFT snapshots.
    ///
    /// Does not call the preflight check.
    ///
    /// # Errors
    ///
    /// Returns an error if the sync directory cannot be retrieved, matching drives cannot be
    /// resolved, or index files cannot be read, built, or written.
    pub fn invoke(drive_infos: Vec<DriveSyncInfo>) -> eyre::Result<()> {
        let sync_phase = SyncPhase::start("sync_index", None);
        let drive_count = drive_infos.len();
        info!(
            "Building search indexes for drives: {}",
            drive_infos.iter().map(|info| info.drive_letter).join(", ")
        );

        for info in drive_infos {
            let _span = info_span!(
                "sync_index_for_drive",
                drive = %info.drive_letter,
                mft_path = %info.mft_output_path.display(),
                index_path = %info.index_output_path.display(),
            )
            .entered();
            Self::invoke_for_mft_path(&info)?;
        }

        let elapsed = sync_phase.elapsed();
        info!(
            phase = sync_phase.name(),
            drive = %sync_phase.drive(),
            drive_count,
            elapsed_ms = elapsed_ms(elapsed),
            elapsed_human = %elapsed_human(elapsed),
            "Finished sync phase"
        );

        Ok(())
    }

    /// Build a search index from an already-loaded in-memory `MftFile`.
    ///
    /// # Errors
    ///
    /// Returns an error if path conversion or index writing fails.
    pub fn invoke_for_mft_file(info: &DriveSyncInfo, mft_file: &MftFile) -> eyre::Result<()> {
        let index_phase = SyncPhase::start("build_search_index", Some(info.drive_letter));
        let source_mft_bytes = u64_from_usize(mft_file.size().get::<byte>());
        let source_mft_entries = mft_file.record_count();
        let rows_phase = SyncPhase::start("build_search_index_rows", Some(info.drive_letter));
        let rows = {
            let _span = info_span!(
                "build_search_index_rows",
                drive = %info.drive_letter,
                mft_size = %mft_file.size().get::<byte>(),
            )
            .entered();
            Self::build_rows_for_mft_file(info, mft_file)?
        };
        let rows_elapsed = rows_phase.elapsed();
        info!(
            phase = rows_phase.name(),
            drive = %rows_phase.drive(),
            elapsed_ms = elapsed_ms(rows_elapsed),
            elapsed_human = %elapsed_human(rows_elapsed),
            source_mft_bytes,
            source_mft_human = %bytes_human(source_mft_bytes),
            source_mft_entries,
            entries_per_second = count_per_second(source_mft_entries, rows_elapsed),
            entries_per_second_human = %count_per_second_human(source_mft_entries, rows_elapsed),
            row_count = rows.len(),
            rows_per_second = count_per_second(rows.len(), rows_elapsed),
            rows_per_second_human = %count_per_second_human(rows.len(), rows_elapsed),
            "Finished sync phase"
        );
        let output_bytes = {
            let _span = info_span!(
                "write_search_index_output",
                drive = %info.drive_letter,
                row_count = rows.len(),
                output_path = %info.index_output_path.display(),
            )
            .entered();
            let write_phase =
                SyncPhase::start("write_search_index_output", Some(info.drive_letter));
            let output_bytes = Self::write_index_output(info, mft_file, &rows)?;
            let write_elapsed = write_phase.elapsed();
            info!(
                phase = write_phase.name(),
                drive = %write_phase.drive(),
                elapsed_ms = elapsed_ms(write_elapsed),
                elapsed_human = %elapsed_human(write_elapsed),
                output_bytes,
                output_bytes_human = %bytes_human(output_bytes),
                bytes_per_second = bytes_per_second(output_bytes, write_elapsed),
                bytes_per_second_human = %bytes_per_second_human(output_bytes, write_elapsed),
                row_count = rows.len(),
                rows_per_second = count_per_second(rows.len(), write_elapsed),
                rows_per_second_human = %count_per_second_human(rows.len(), write_elapsed),
                output_path = %info.index_output_path.display(),
                "Finished sync phase"
            );
            output_bytes
        };
        let index_elapsed = index_phase.elapsed();
        info!(
            phase = index_phase.name(),
            drive = %index_phase.drive(),
            elapsed_ms = elapsed_ms(index_elapsed),
            elapsed_human = %elapsed_human(index_elapsed),
            source_mft_bytes,
            source_mft_human = %bytes_human(source_mft_bytes),
            source_mft_entries,
            entries_per_second = count_per_second(source_mft_entries, index_elapsed),
            entries_per_second_human = %count_per_second_human(source_mft_entries, index_elapsed),
            row_count = rows.len(),
            rows_per_second = count_per_second(rows.len(), index_elapsed),
            rows_per_second_human = %count_per_second_human(rows.len(), index_elapsed),
            output_bytes,
            output_bytes_human = %bytes_human(output_bytes),
            "Finished sync phase"
        );
        Ok(())
    }

    pub(crate) fn invoke_for_mft_path(info: &DriveSyncInfo) -> eyre::Result<()> {
        if !info.mft_output_path.is_file() {
            bail!(
                "Cannot build index for drive {}: missing {}",
                info.drive_letter,
                info.mft_output_path.display()
            );
        }

        let load_phase = SyncPhase::start("load_cached_mft_for_index", Some(info.drive_letter));
        let mft_file = {
            let _span = info_span!(
                "load_cached_mft_for_index",
                drive = %info.drive_letter,
                mft_path = %info.mft_output_path.display(),
            )
            .entered();
            MftFile::from_path(&info.mft_output_path).wrap_err_with(|| {
                format!(
                    "Failed parsing MFT snapshot for drive {} from {}",
                    info.drive_letter,
                    info.mft_output_path.display()
                )
            })?
        };
        let load_elapsed = load_phase.elapsed();
        let source_mft_bytes = u64_from_usize(mft_file.size().get::<byte>());
        let source_mft_entries = mft_file.record_count();
        info!(
            phase = load_phase.name(),
            drive = %load_phase.drive(),
            elapsed_ms = elapsed_ms(load_elapsed),
            elapsed_human = %elapsed_human(load_elapsed),
            source_mft_bytes,
            source_mft_human = %bytes_human(source_mft_bytes),
            source_mft_entries,
            bytes_per_second = bytes_per_second(source_mft_bytes, load_elapsed),
            bytes_per_second_human = %bytes_per_second_human(source_mft_bytes, load_elapsed),
            entries_per_second = count_per_second(source_mft_entries, load_elapsed),
            entries_per_second_human = %count_per_second_human(source_mft_entries, load_elapsed),
            "Finished sync phase"
        );

        Self::invoke_for_mft_file(info, &mft_file)
    }

    fn build_rows_for_mft_file(
        info: &DriveSyncInfo,
        mft_file: &MftFile,
    ) -> eyre::Result<Vec<SearchIndexPathRow>> {
        let drive_name = info.drive_letter.to_string();

        let files =
            convert_mft_file_to_path_collection(&drive_name, mft_file).wrap_err_with(|| {
                format!(
                    "Failed processing MFT data for drive {} from {}",
                    info.drive_letter,
                    info.mft_output_path.display()
                )
            })?;

        Ok(files
            .0
            .into_iter()
            .flatten()
            .map(|path| SearchIndexPathRow {
                path: path.path.to_string_lossy().into_owned(),
                has_deleted_entries: path.has_deleted_entries(),
            })
            .collect())
    }

    fn write_index_output(
        info: &DriveSyncInfo,
        mft_file: &MftFile,
        rows: &[SearchIndexPathRow],
    ) -> eyre::Result<u64> {
        SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new(
                info.drive_letter,
                u64_from_usize(mft_file.size().get::<byte>()),
                u64_from_usize(rows.len()),
            ),
            rows,
        )?
        .write_to_path(&info.index_output_path)
        .wrap_err_with(|| {
            format!(
                "Failed writing index output for drive {} to {}",
                info.drive_letter,
                info.index_output_path.display()
            )
        })?;

        Ok(info
            .index_output_path
            .metadata()
            .wrap_err_with(|| {
                format!(
                    "Failed reading metadata for index output {}",
                    info.index_output_path.display()
                )
            })?
            .len())
    }
}
