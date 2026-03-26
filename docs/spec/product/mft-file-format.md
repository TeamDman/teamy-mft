# MFT File Format

This specification describes the assumptions `teamy-mft` makes about the logical Master File Table stream it reads from disk or from cached `.mft` files.

## Cached Logical Stream

mftf[cached-stream.begins-at-record-zero]
The cached logical MFT byte stream must begin directly with MFT record 0, without any file-level header before the first `FILE` record.

mftf[cached-stream.record-size-field]
The cached logical MFT byte stream must expose the per-record size in the first record at offset `0x1C`.

mftf[cached-stream.fixed-record-size]
The cached logical MFT byte stream must contain fixed-size records whose size evenly divides the full buffer length.

mftf[cached-stream.fixups-applied-before-iteration]
Before the application iterates cached MFT records, update-sequence-array fixups must have been applied to the logical stream.

## Record Iteration

mftf[record-iteration.contiguous-fixed-size-slices]
The application assumes cached MFT records can be iterated as contiguous fixed-size slices of the logical byte stream.

## Attribute Iteration

mftf[attribute-iteration.bounded-by-used-size]
The application assumes the attribute list for a record is bounded by the record's used-size field and must not read beyond that boundary.

mftf[attribute-iteration.ends-at-sentinel]
The application assumes the attribute list terminates at the NTFS end-of-attributes sentinel or at the first invalid attribute boundary.

## File Name And Paths

mftf[file-name-attributes.resident-x30]
The application's fast filename extraction path assumes file names are read from resident `FILE_NAME` (`0x30`) attributes contained within record bounds.

mftf[path-resolution.parent-chain-absolute-paths]
The application assumes parent references from `FILE_NAME` attributes can be followed to reconstruct absolute paths rooted at the selected drive prefix.