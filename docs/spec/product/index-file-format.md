# Index File Format

This specification covers the cached search index artifact written and read by `teamy-mft`.

## Search Index Header

idxf[search-index.header.magic]
The serialized search index header must begin with a fixed magic value that identifies the file as a `teamy-mft` search index.

idxf[search-index.header.version]
The serialized search index header must store an explicit format version.

idxf[search-index.header.drive-letter]
The serialized search index header must store the drive letter associated with the indexed data.

idxf[search-index.header.source-mft-length]
The serialized search index header must store the source MFT length in bytes.

idxf[search-index.header.node-count]
The serialized search index header must store the number of indexed path rows.

## Search Index Rows

idxf[search-index.row.deleted-flag]
Each indexed path row must preserve whether the reconstructed path includes deleted MFT entries.

## Compatibility

idxf[search-index.reader.rejects-unsupported-version]
The search index reader must reject unsupported on-disk format versions and direct the user to rebuild the stale index.