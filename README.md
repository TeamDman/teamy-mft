# Teamy MFT

Command-line toolkit for interacting with the NTFS Master File Table on Windows.

## Installation

```bash
cargo install --path .
```

## Commands (initial)

```
Teamy MFT commands

Usage: teamy-mft.exe [OPTIONS] <COMMAND>

Commands:
  sync               Sync operations (requires elevation)
  list-paths         Produce newline-delimited list of file paths for matching drives from cached .mft files
  get-sync-dir       Get the currently configured sync directory
  set-sync-dir       Set the sync directory (defaults to current directory if omitted)
  check              Validate cached MFT files have at least one Win32 FILE_NAME attribute per entry having any FILE_NAME
  query              Query resolved file paths (substring match) across cached MFTs
  robocopy-logs-tui  Explore robocopy logs in a TUI (validate file exists for now)
  help               Print this message or the help of the given subcommand(s)

Options:
      --debug    Enable debug logging
  -h, --help     Print help
  -V, --version  Print version
```

## Links

For parsing discontiguous MFT

https://github.com/pitest3141592653/sysMFT/blob/e0b60a040ccdd07337a9715777e455a82f64b216/main.py

https://www.futurelearn.com/info/courses/introduction-to-malware-investigations/0/steps/146529

https://learn.microsoft.com/en-us/windows/win32/fileio/master-file-table

https://learn.microsoft.com/en-us/windows/win32/devnotes/master-file-table

https://learn.microsoft.com/en-us/troubleshoot/windows-server/backup-and-storage/ntfs-reserves-space-for-mft

https://github.com/libyal/libfsntfs/blob/82181db7c9f272f98257cf3576243d9ccbbe8823/documentation/New%20Technologies%20File%20System%20(NTFS).asciidoc

https://digitalinvestigator.blogspot.com/2022/03/the-ntfs-master-file-table-mft.html?m=1

https://ntfs.com/ntfs-mft.htm

https://web.archive.org/web/20230104064834/http://inform.pucp.edu.pe/~inf232/Ntfs/ntfs_doc_v0.5/concepts/data_runs.html

https://www.disk-editor.org/index.html#features

[The NTFS Master File Table (MFT)](https://digitalinvestigator.blogspot.com/2022/03/the-ntfs-master-file-table-mft.html?m=1)

[NTFS Partition Boot Sector - NTFS.com](https://ntfs.com/ntfs-partition-boot-sector.htm)

[sysMFT/main.py at e0b60a040ccdd07337a9715777e455a82f64b216 · pitest3141592653/sysMFT](https://github.com/pitest3141592653/sysMFT/blob/e0b60a040ccdd07337a9715777e455a82f64b216/main.py)

[Data Runs - Concept - NTFS Documentation](https://web.archive.org/web/20230104064834/http://inform.pucp.edu.pe/~inf232/Ntfs/ntfs_doc_v0.5/concepts/data_runs.html)

[FILE_RECORD_SEGMENT_HEADER structure - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/devnotes/file-record-segment-header)

[MULTI_SECTOR_HEADER structure - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/devnotes/multi-sector-header)

[ATTRIBUTE_LIST_ENTRY structure - Win32 apps | Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/devnotes/attribute-list-entry)

[Jonathan Adkins - The Master File Table Lecture Video Part 1](https://www.youtube.com/watch?v=q3_V0EJcD-k)

[Jonathan Adkins - The Master File Table Lecture Video Part 2](https://www.youtube.com/watch?v=gKDJLa0OoDc)

[Jonathan Adkins - The Master File Table Lecture Video Part 3](https://www.youtube.com/watch?v=GHLwl77b36s)