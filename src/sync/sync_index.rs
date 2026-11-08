use crate::machine::DriveArtifactInfo;
use crate::mft::mft_convert_to_path_collection::convert_mft_file_to_path_collection;
use crate::mft::mft_file::MftFile;
use crate::search_index::format::SearchIndexHeader;
use crate::search_index::format::SearchIndexPathRow;
use crate::search_index::search_index_bytes::SearchIndexBytesMut;
use eyre::Context;
use eyre::bail;
use std::time::Instant;
use teamy_cancellation::CancellationScope;
use tracing::info;
use tracing::info_span;

#[derive(Debug)]
pub(super) struct SyncIndex;

impl SyncIndex {
    pub(super) fn build(info: &DriveArtifactInfo, cancel: &CancellationScope) -> eyre::Result<()> {
        if !info.mft_path.is_file() {
            bail!(
                "Cannot build index for drive {}: missing {}",
                info.drive_letter,
                info.mft_path.display()
            );
        }

        let start = Instant::now();
        info!(drive_letter = %info.drive_letter, mft_path = %info.mft_path.display(), "Building search index from cached snapshot");
        let mft_file = {
            let _span = info_span!(
                "load_cached_mft_for_index",
                drive = %info.drive_letter,
                mft_path = %info.mft_path.display(),
            )
            .entered();
            MftFile::from_path(&info.mft_path, cancel).wrap_err_with(|| {
                format!(
                    "Failed parsing MFT snapshot for drive {} from {}",
                    info.drive_letter,
                    info.mft_path.display()
                )
            })?
        };

        let mft_size = mft_file.size();
        let files = convert_mft_file_to_path_collection(info.drive_letter, &mft_file)
            .wrap_err_with(|| {
                format!(
                    "Failed processing MFT data for drive {} from {}",
                    info.drive_letter,
                    info.mft_path.display()
                )
            })?;

        let rows = files
            .0
            .into_iter()
            .flatten()
            .map(|path| SearchIndexPathRow {
                path: path.path.to_string_lossy().into_owned().into(),
                has_deleted_entries: path.has_deleted_entries(),
            })
            .collect::<Vec<_>>();
        drop(mft_file);
        eyre::ensure!(
            !cancel.is_cancelled(),
            "Cancelled before publishing index for drive {}",
            info.drive_letter
        );
        SearchIndexBytesMut::from_owned_rows(
            SearchIndexHeader::new(info.drive_letter, mft_size, u64::try_from(rows.len())?),
            rows,
        )?
        .write_to_path(&info.base_index_path)
        .wrap_err_with(|| {
            format!(
                "Failed writing index output for drive {} to {}",
                info.drive_letter,
                info.base_index_path.display()
            )
        })?;
        info!(drive_letter = %info.drive_letter, elapsed_ms = start.elapsed().as_millis(), "Cached search index build finished");
        Ok(())
    }
}
