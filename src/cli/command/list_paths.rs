use crate::drive_letter_pattern::DriveLetterPattern;
use crate::mft::mft_file::MftFile;
use crate::sync_dir::try_get_sync_dir;
use arbitrary::Arbitrary;
use clap::Args;
use eyre::Context;
use mft::FileNameAttr;
use mft::MftParser;
use mft::attribute::MftAttributeContent;
use mft::attribute::x30::FileNamespace;
use rustc_hash::FxHashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::Instant;
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

        for mft_file_path in &mft_files {
            let mft_file = MftFile::from_path(mft_file_path)?;
            let mft_bytes: &[u8] = &mft_file;
            info!("Loaded MFT file: {}", mft_file_path.display());

            info!("Parsing MFT file: {}", mft_file_path.display());
            let start = Instant::now();
            let mut parser =
                MftParser::from_read_seek(Cursor::new(mft_bytes), Some(mft_bytes.len() as u64))
                    .wrap_err_with(|| {
                        format!("Failed to parse MFT bytes from {}", mft_file_path.display())
                    })?;

            // Collect canonical FILE_NAME (x30) attributes per MFT entry.
            // For each (parent, name) pair keep only highest precedence namespace.
            let mut x30_map = FxHashMap::<MftReference, Vec<FileNameAttr>>::default();
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
                        warn!("Failed to parse entry from {}: {}", mft_file_path.display(), e);
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
                    let list = x30_map.entry(key).or_default();
                    if let Some(existing) = list
                        .iter_mut()
                        .find(|f| f.parent == x30.parent && f.name == x30.name)
                    {
                        let existing_rank = prec_index(&existing.namespace);
                        let new_rank = prec_index(&x30.namespace);
                        if new_rank < existing_rank {
                            *existing = x30; // better namespace precedence
                        } else if new_rank == existing_rank {
                            warn!(
                                "Duplicate FILE_NAME same precedence for {:?} parent {:?} name {:?} {:?}",
                                key, x30.parent, x30.name, x30.namespace
                            );
                        }
                        // lower precedence ignored
                    } else {
                        list.push(x30);
                    }
                }
            }
            let elapsed = start.elapsed();
            let entry_count = x30_map.len();
            let link_count: usize = x30_map.values().map(|v| v.len()).sum();
            info!(
                "Indexed {} MFT entries ({} canonical FILE_NAME links) in {:.2?}",
                entry_count, link_count, elapsed
            );

            const ROOT_ENTRY: u64 = 5;
            fn choose_dir<'a>(
                links: &'a [FileNameAttr],
                prec_index: &impl Fn(&FileNamespace) -> usize,
            ) -> &'a FileNameAttr {
                links
                    .iter()
                    .min_by_key(|f| (prec_index(&f.namespace), f.name.len()))
                    .expect("links not empty")
            }

            for (entry_ref, links) in x30_map.iter() {
                if entry_ref.entry == ROOT_ENTRY {
                    continue;
                }
                let mut seen = std::collections::HashSet::<String>::new();
                for link in links {
                    // one output per hard link
                    // Build path components
                    let mut components: Vec<&str> = Vec::new();
                    components.push(&link.name);
                    let mut parent_ref = link.parent;
                    while parent_ref.entry != ROOT_ENTRY {
                        if let Some(parent_links) = x30_map.get(&parent_ref) {
                            let parent_attr = choose_dir(parent_links, &prec_index);
                            components.push(parent_attr.name.as_str());
                            parent_ref = parent_attr.parent;
                        } else {
                            break;
                        }
                    }
                    let mut full = String::new();
                    for comp in components.iter().rev() {
                        full.push('\\');
                        full.push_str(comp);
                    }
                    if seen.insert(full.clone()) {
                        println!("{full}");
                    }
                }
            }
        }
        Ok(())
    }
}

impl crate::cli::to_args::ToArgs for ListPathsArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        let mut args = Vec::new();
        // drive_pattern is a positional argument with a default of "*".
        // Only include it when it's not the default so that roundtrip
        // parsing reproduces the same value and we don't add redundant args.
        if self.drive_pattern != DriveLetterPattern::default() {
            args.push(self.drive_pattern.to_string().into());
        }
        args
    }
}
