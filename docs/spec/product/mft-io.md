# MFT File Reading And Writing

This specification covers implementation-level NTFS and MFT read behavior that is distinct from the serialized search index format.

## Boot Sector

mfti[boot-sector.reads-512-byte-sector]
Boot sector reads must load the 512-byte NTFS boot sector from offset 0 of the selected drive handle.

mfti[boot-sector.file-record-size-encoding]
Boot sector parsing must interpret the NTFS file record size encoding correctly for both negative exponent and positive cluster-count representations.

## Fixups

mfti[fixup.detects-needed-fixup]
The fixup pipeline must detect whether an MFT entry still requires update-sequence-array fixup application.

mfti[fixup.applies-in-place]
The fixup pipeline must apply update-sequence-array fixups in place when the entry requires them.

mfti[fixup.parallel-buffer-processing]
The fixup pipeline must support applying fixups across an aligned MFT buffer in parallel.