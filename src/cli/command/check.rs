use crate::drive_letter_pattern::DriveLetterPattern;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use eyre::Context;
use memmap2::Mmap;
use mft::MftParser;
use mft::attribute::MftAttributeContent;
use mft::attribute::x30::FileNamespace;
use std::fs::File;
use std::io::Cursor;
use std::io::Write;
use std::io::{self};
use std::path::PathBuf;
use tracing::error;
use tracing::info;
use tracing::warn;
use winstructs::ntfs::mft_reference::MftReference;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct CheckArgs {
    /// Drive letter pattern to match drives whose cached MFTs will be checked (e.g., "*", "C", "CD", "C,D")
    #[clap(default_value_t = DriveLetterPattern::default())]
    pub drive_pattern: DriveLetterPattern,
    /// Fail fast on first violation (otherwise report all)
    #[clap(long, default_value_t = false)]
    pub fail_fast: bool,
}

impl CheckArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        let sync_dir = try_get_sync_dir()?;

        println!(
            "This check assumed that each entry would have a Win32 name. This is not the case. Proceed at your own volition, but the errors below aren't necessarily a problem."
        );
        print!("Press Enter to continue...");
        io::stdout().flush().ok();
        let _ = io::stdin().read_line(&mut String::new());

        let drive_letters = self.drive_pattern.into_drive_letters()?;
        let mft_files: Vec<PathBuf> = drive_letters
            .into_iter()
            .map(|d| sync_dir.join(format!("{d}.mft")))
            .filter(|p| p.is_file())
            .collect();

        let mut violations = 0usize;

        for path in &mft_files {
            info!("Checking MFT file: {}", path.display());
            let file =
                File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
            let mmap = unsafe { Mmap::map(&file) }
                .with_context(|| format!("Failed to memory-map {}", path.display()))?;
            let mft_bytes: &[u8] = &mmap;

            let mut parser =
                MftParser::from_read_seek(Cursor::new(mft_bytes), Some(mft_bytes.len() as u64))
                    .wrap_err_with(|| {
                        format!("Failed to parse MFT bytes from {}", path.display())
                    })?;

            for entry in parser.iter_entries() {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("Failed to parse entry from {}: {}", path.display(), e);
                        continue;
                    }
                };
                let mut has_any = false;
                let mut has_win32 = false;
                let mut namespaces: Vec<(FileNamespace, String)> = Vec::new();
                for attr in entry.iter_attributes().filter_map(|a| a.ok()) {
                    if let MftAttributeContent::AttrX30(x30) = attr.data {
                        has_any = true;
                        if matches!(
                            x30.namespace,
                            FileNamespace::Win32 | FileNamespace::Win32AndDos
                        ) {
                            has_win32 = true;
                        }
                        namespaces.push((x30.namespace, x30.name.to_string()));
                    }
                }
                if has_any && !has_win32 {
                    violations += 1;
                    error!(
                        "Entry {:?} in {} has FILE_NAME but no Win32 namespace. Namespaces present: {:?}",
                        MftReference {
                            entry: entry.header.record_number,
                            sequence: entry.header.sequence
                        },
                        path.display(),
                        namespaces
                            .iter()
                            .map(|(ns, name)| format!("{:?}:{}", ns, name))
                            .collect::<Vec<_>>()
                    );
                    if self.fail_fast {
                        break;
                    }
                }
            }

            if self.fail_fast && violations > 0 {
                break;
            }
        }

        if violations > 0 {
            eyre::bail!("{} entries missing Win32 FILE_NAME attribute", violations);
        }
        info!("All entries OK (every entry with any FILE_NAME has a Win32 namespace)");
        Ok(())
    }
}

impl crate::cli::to_args::ToArgs for CheckArgs {}
