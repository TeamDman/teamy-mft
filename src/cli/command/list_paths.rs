use crate::drive_letter_pattern::DriveLetterPattern;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use eyre::Context;
use memmap2::Mmap;
use mft::FileNameAttr;
use mft::MftParser;
use mft::attribute::MftAttributeContent;
use mft::attribute::x30::FileNamespace;
use rustc_hash::FxHashMap;
use tracing::trace;
use std::collections::VecDeque;
use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;
use tracing::info;
use tracing::warn;
use winstructs::ntfs::mft_reference::MftReference;

#[derive(Args, Arbitrary, PartialEq, Debug, Default)]
pub struct ListPathsArgs {
    /// Drive letter pattern to match drives whose cached MFTs will be traversed (e.g., "*", "C", "CD", "C,D")
    #[clap(default_value_t = DriveLetterPattern::default())]
    pub drive_pattern: DriveLetterPattern,
}

impl ListPathsArgs {
    pub fn invoke(self) -> eyre::Result<()> {
        // Determine sync dir
        let sync_dir = try_get_sync_dir()?;
        // Resolve drive letters from pattern
        let drive_letters = self.drive_pattern.into_drive_letters()?;
        // Build list of existing cached MFT files for matching drives
        let mft_files: Vec<PathBuf> = drive_letters
            .into_iter()
            .map(|d| sync_dir.join(format!("{d}.mft")))
            .filter(|p| p.is_file())
            .collect();

        for path in &mft_files {
            info!("Loading MFT file: {}", path.display());
            let file =
                File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
            // Safety: mapping read-only; file descriptor lives until mmap dropped in loop iteration.
            let mmap = unsafe { Mmap::map(&file) }
                .with_context(|| format!("Failed to memory-map {}", path.display()))?;
            let mft_bytes: &[u8] = &mmap; // bytes of the MFT file
            info!("Loaded MFT file: {}", path.display());

            info!("Parsing MFT file: {}", path.display());
            let mut parser =
                MftParser::from_read_seek(Cursor::new(mft_bytes), Some(mft_bytes.len() as u64))
                    .wrap_err_with(|| {
                        format!("Failed to parse MFT bytes from {}", path.display())
                    })?;

            // Map each MFT reference to its canonical FILE_NAME (x30) attribute chosen by precedence
            let mut x30_map = FxHashMap::<MftReference, (FileNamespace, FileNameAttr)>::default();
            // Lower index => higher precedence
            let precedence = [
                FileNamespace::Win32,
                FileNamespace::Win32AndDos,
                FileNamespace::POSIX,
                FileNamespace::DOS,
            ];
            let prec_index = |ns: &FileNamespace| {
                precedence
                    .iter()
                    .position(|p| p == ns)
                    .unwrap_or(precedence.len())
            };

            for entry in parser.iter_entries() {
                let entry = match entry {
                    Ok(x) => x,
                    Err(e) => {
                        warn!("Failed to parse entry from {}: {}", path.display(), e);
                        continue;
                    }
                };

                for x30 in entry
                    .iter_attributes()
                    .filter_map(|attr| attr.ok())
                    .filter_map(|attr| match attr.data {
                        MftAttributeContent::AttrX30(data) => Some(data),
                        _ => None,
                    })
                {
                    let key = MftReference {
                        entry: entry.header.record_number,
                        sequence: entry.header.sequence,
                    };
                    match x30_map.get_mut(&key) {
                        Some((existing_ns, existing_attr)) => {
                            let existing_rank = prec_index(existing_ns);
                            let new_rank = prec_index(&x30.namespace);
                            if new_rank < existing_rank {
                                // better precedence
                                trace!(
                                    "Replacing FILE_NAME for {:?} in {}: {:?} -> {:?}",
                                    key,
                                    path.display(),
                                    existing_ns,
                                    x30.namespace
                                );
                                *existing_ns = x30.namespace;
                                *existing_attr = x30;
                            } else if new_rank == existing_rank {
                                warn!(
                                    "Duplicate FILE_NAME same precedence for {:?} ({:?}) in {}:\nold={:#?}\nnew={:#?}",
                                    key,
                                    existing_ns,
                                    path.display(),
                                    existing_attr,
                                    x30
                                );
                            } else {
                                // lower precedence, ignore
                            }
                        }
                        None => {
                            x30_map.insert(key, (x30.namespace, x30));
                        }
                    }
                }
            }

            const ROOT_ENTRY: u64 = 5;
            for (_, (_, entry_attr)) in x30_map.iter() {
                let mut path = VecDeque::new();
                path.push_back(&entry_attr.name);
                let mut parent_nav = &entry_attr.parent;
                while parent_nav.entry != ROOT_ENTRY {
                    if let Some((_, parent_attr)) = x30_map.get(parent_nav) {
                        path.push_front(&parent_attr.name);
                        parent_nav = &parent_attr.parent;
                        continue;
                    }
                    break;
                }
                for part in &path {
                    print!("\\{}", part);
                }
                println!();
            }
        }
        Ok(())
    }
}

impl crate::cli::to_args::ToArgs for ListPathsArgs {}
