use crate::paths::EnsureParentDirExists;
use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};
use std::path::PathBuf;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct MoveArgs {
    /// Move this exact file path
    #[facet(args::positional)]
    pub source: String,
    /// Existing destination directory, directory path ending with a slash, or exact destination file path
    #[facet(args::positional)]
    pub destination: String,
}

impl MoveArgs {
    /// # Errors
    ///
    /// Returns an error if the source file does not exist, the destination cannot be resolved,
    /// the move would overwrite an existing file, or the underlying rename fails.
    pub fn invoke(self) -> eyre::Result<()> {
        let source = self.source.trim();
        if source.is_empty() {
            eyre::bail!("source file path must not be empty");
        }
        let destination = self.destination.trim();
        if destination.is_empty() {
            eyre::bail!("destination path must not be empty");
        }

        let source_path = PathBuf::from(source);
        if !source_path.is_file() {
            eyre::bail!("Source file {} does not exist", source_path.display());
        }
        let source_file_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| eyre::eyre!("{}: invalid UTF-8 filename", source_path.display()))?;

        let destination_input = PathBuf::from(destination);
        let destination_path = if destination_input.is_dir()
            || destination.ends_with('\\')
            || destination.ends_with('/')
        {
            destination_input.join(source_file_name)
        } else {
            destination_input
        };
        if source_path == destination_path {
            println!("File already at {}", destination_path.display());
            return Ok(());
        }
        if destination_path.exists() {
            eyre::bail!(
                "Destination file {} already exists",
                destination_path.display()
            );
        }

        destination_path.ensure_parent_dir_exists()?;
        std::fs::rename(&source_path, &destination_path).map_err(|error| {
            eyre::eyre!(
                "Failed moving {} to {}: {}",
                source_path.display(),
                destination_path.display(),
                error
            )
        })?;
        println!(
            "Moved {} -> {}",
            source_path.display(),
            destination_path.display()
        );

        let sync_dir = crate::machine::config::load_sync_dir_from_config()?;
        let rendered_source_path = source_path.to_string_lossy().into_owned();
        match crate::sync::sync_path_into_published_overlay(&sync_dir, &rendered_source_path) {
            Ok(drive_letter) => {
                println!(
                    "Ran `teamy-mft sync {}` automatically and updated the published overlay for drive {}.",
                    source_path.display(),
                    drive_letter
                );
            }
            Err(error) => {
                println!(
                    "Tried running `teamy-mft sync {}` automatically, but it failed: {}",
                    source_path.display(),
                    error
                );
            }
        }

        let rendered_destination_path = destination_path.to_string_lossy().into_owned();
        match crate::sync::sync_path_into_published_overlay(&sync_dir, &rendered_destination_path) {
            Ok(drive_letter) => {
                println!(
                    "Ran `teamy-mft sync {}` automatically and updated the published overlay for drive {}.",
                    destination_path.display(),
                    drive_letter
                );
            }
            Err(error) => {
                println!(
                    "Tried running `teamy-mft sync {}` automatically, but it failed: {}",
                    destination_path.display(),
                    error
                );
            }
        }

        Ok(())
    }
}
