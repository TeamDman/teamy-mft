# Teamy MFT

Command-line toolkit for upcoming MFT / storage utilities.

## Installation

```bash
cargo install --path .
```

## Commands (initial)

- `teamy-mft get-sync-dir` – Prints currently configured sync directory or `<not set>`.
- `teamy-mft set-sync-dir [path]` – Sets sync directory. If `path` omitted uses current working directory.

Configuration stored per-user in platform config directory (JSON file `sync_dir.json`).

## Roadmap

- Integrate existing MFT parsing library
- Add dump/query/diff/show subcommands
- Sync workflows leveraging configured directory


## Links

For parsing discontiguous MFT

https://github.com/pitest3141592653/sysMFT/blob/e0b60a040ccdd07337a9715777e455a82f64b216/main.py

https://www.futurelearn.com/info/courses/introduction-to-malware-investigations/0/steps/146529

https://learn.microsoft.com/en-us/windows/win32/fileio/master-file-table

https://learn.microsoft.com/en-us/windows/win32/devnotes/master-file-table

https://learn.microsoft.com/en-us/troubleshoot/windows-server/backup-and-storage/ntfs-reserves-space-for-mft

https://github.com/libyal/libfsntfs/blob/82181db7c9f272f98257cf3576243d9ccbbe8823/documentation/New%20Technologies%20File%20System%20(NTFS).asciidoc

https://digitalinvestigator.blogspot.com/2022/03/the-ntfs-master-file-table-mft.html?m=1

https://ntfs.com/ntfs-partition-boot-sector.htm

https://ntfs.com/ntfs-mft.htm

https://web.archive.org/web/20230104064834/http://inform.pucp.edu.pe/~inf232/Ntfs/ntfs_doc_v0.5/concepts/data_runs.html