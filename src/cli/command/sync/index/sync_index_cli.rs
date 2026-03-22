use crate::cli::command::sync::IfExistsOutputBehaviour;
use crate::cli::command::sync::drive_sync_info::DriveSyncInfo;
use crate::mft::mft_convert_to_path_collection::convert_mft_file_to_path_collection;
use crate::mft::mft_file::MftFile;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use arbitrary::Arbitrary;
use eyre::Context;
use eyre::bail;
use facet::Facet;
use itertools::Itertools;
use tracing::debug;
use tracing::info;
use uom::si::information::byte;

#[derive(Facet, PartialEq, Debug, Arbitrary, Default, Clone)]
pub struct SyncIndexArgs;

impl SyncIndexArgs {
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
    pub fn invoke(&self, drive_infos: Vec<DriveSyncInfo>) -> eyre::Result<()> {
        info!(
            "Building search indexes for drives: {}",
            drive_infos.iter().map(|info| info.drive_letter).join(", ")
        );

        for info in drive_infos {
            self.invoke_for_mft_path(&info)?;
        }

        info!("Index sync stage completed");

        Ok(())
    }

    /// Build a search index from an already-loaded in-memory `MftFile`.
    ///
    /// # Errors
    ///
    /// Returns an error if path conversion or index writing fails.
    pub fn invoke_for_mft_file(
        &self,
        info: &DriveSyncInfo,
        mft_file: &MftFile,
    ) -> eyre::Result<()> {
        let rows = Self::build_rows_for_mft_file(info, mft_file)?;
        Self::write_index_output(info, mft_file, &rows)
    }

    fn invoke_for_mft_path(&self, info: &DriveSyncInfo) -> eyre::Result<()> {
        if !info.mft_output_path.is_file() {
            bail!(
                "Cannot build index for drive {}: missing {}",
                info.drive_letter,
                info.mft_output_path.display()
            );
        }

        let mft_file = MftFile::from_path(&info.mft_output_path).wrap_err_with(|| {
            format!(
                "Failed parsing MFT snapshot for drive {} from {}",
                info.drive_letter,
                info.mft_output_path.display()
            )
        })?;

        self.invoke_for_mft_file(info, &mft_file)
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
    ) -> eyre::Result<()> {
        SearchIndexHeader::new(
            info.drive_letter,
            mft_file.size().get::<byte>() as u64,
            rows.len() as u64,
        )
        .write_to_path(&info.index_output_path, rows)
        .wrap_err_with(|| {
            format!(
                "Failed writing index output for drive {} to {}",
                info.drive_letter,
                info.index_output_path.display()
            )
        })
    }
}
