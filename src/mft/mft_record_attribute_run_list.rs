use crate::mft::mft_record::MftRecord;
use crate::read::logical_read_plan::LogicalFileSegment;
use crate::read::logical_read_plan::LogicalFileSegmentKind;
use crate::read::logical_read_plan::LogicalReadPlan;
use eyre::Context;
use eyre::Result;
use eyre::eyre;
use std::collections::BTreeSet;
use std::ops::Deref;
use std::ops::DerefMut;
use tracing::instrument;
use uom::ConstZero;
use uom::si::usize::Information;

/// Generic decoded run list entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MftRecordAttributeRunListEntry {
    /// How many clusters this run occupies
    pub length_clusters: u64,
    /// If none, the run is sparse
    pub local_cluster_network_start_entry_index: Option<u64>,
}

/// Borrowed view over an encoded NTFS run list (sequence of data runs) used by any non-resident attribute.
#[derive(Debug, Clone, Copy)]
pub struct MftRecordAttributeRunList<'a> {
    raw: &'a [u8],
}

impl<'a> MftRecordAttributeRunList<'a> {
    #[must_use]
    pub fn new(raw: &'a [u8]) -> Self {
        Self { raw }
    }
    #[must_use]
    pub fn as_slice(&self) -> &'a [u8] {
        self.raw
    }
    #[must_use]
    pub fn iter(&self) -> MftRecordAttributeRunListIter<'a> {
        MftRecordAttributeRunListIter {
            raw: self.raw,
            pos: 0,
            last_lcn: 0,
        }
    }
    /// # Errors
    ///
    /// Returns an error if the runlist data is invalid.
    pub fn decode_all(&self) -> Result<Vec<MftRecordAttributeRunListEntry>> {
        self.iter().collect()
    }
}

impl<'a> IntoIterator for &MftRecordAttributeRunList<'a> {
    type Item = Result<MftRecordAttributeRunListEntry>;
    type IntoIter = MftRecordAttributeRunListIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug)]
pub struct MftRecordAttributeRunListIter<'a> {
    raw: &'a [u8],
    pos: usize,
    last_lcn: i64,
}

impl Iterator for MftRecordAttributeRunListIter<'_> {
    type Item = Result<MftRecordAttributeRunListEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.raw.len() {
            return None;
        }
        let header = self.raw[self.pos];
        let mut pos = self.pos + 1;
        // An invalid run ends the iterator; its following bytes are not another header.
        self.pos = self.raw.len();
        if header == 0 {
            return None;
        }
        let offset_size = (header & 0xF0) >> 4;
        let length_size = header & 0x0F;
        if length_size == 0 {
            return Some(Err(eyre!("Zero length_size in run header")));
        }
        if length_size > 8 || offset_size > 8 {
            return Some(Err(eyre!("Run field width exceeds 8 bytes")));
        }
        if pos + length_size as usize > self.raw.len() {
            return Some(Err(eyre!("Run length field exceeds buffer")));
        }
        let mut length = 0u64;
        for i in 0..length_size {
            length |= u64::from(self.raw[pos + i as usize]) << (8 * i);
        }
        if length == 0 {
            return Some(Err(eyre!("Zero cluster length in data run")));
        }
        pos += length_size as usize;
        let local_cluster_network_start_entry_index = if offset_size == 0 {
            None
        } else {
            if pos + offset_size as usize > self.raw.len() {
                return Some(Err(eyre!("Run offset field exceeds buffer")));
            }
            let offset = &self.raw[pos..pos + offset_size as usize];
            let mut delta = [if offset[offset.len() - 1] & 0x80 == 0 {
                0
            } else {
                0xff
            }; 8];
            delta[..offset.len()].copy_from_slice(offset);
            let Some(lcn) = self.last_lcn.checked_add(i64::from_le_bytes(delta)) else {
                return Some(Err(eyre!("Run cluster offset overflow")));
            };
            let Ok(lcn_unsigned) = u64::try_from(lcn) else {
                return Some(Err(eyre!("Run cluster offset is negative")));
            };
            pos += offset_size as usize;
            self.last_lcn = lcn;
            Some(lcn_unsigned)
        };
        self.pos = pos;
        Some(Ok(MftRecordAttributeRunListEntry {
            length_clusters: length,
            local_cluster_network_start_entry_index,
        }))
    }
}

#[derive(Debug, Default)]
pub struct MftRecordAttributeRunListOwned {
    inner: Vec<MftRecordAttributeRunListEntry>,
}
impl MftRecordAttributeRunListOwned {
    /// Extract the data runs from the unnamed x80 attribute
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid run list instead of returning a partial MFT map.
    #[instrument(skip_all)]
    pub fn from_mft_record(dollar_mft_record: &MftRecord) -> Result<Self> {
        let mut rtn = Self::default();
        for attr in dollar_mft_record.iter_attributes() {
            if attr.get_name_len() != 0 || !attr.get_is_non_resident() {
                continue;
            }
            if let Some(x80) = attr.as_x80() {
                let runlist = x80
                    .get_data_run_list()
                    .wrap_err("Failed to get $MFT data runs")?;
                rtn.extend(
                    runlist
                        .decode_all()
                        .wrap_err("Failed decoding $MFT data run list")?,
                );
            }
        }
        Ok(rtn)
    }
}

impl MftRecordAttributeRunListOwned {
    /// Build a logical plan preserving sparse runs.
    ///
    /// Future opportunity: produce a sparse output file
    /// by marking destination with `FSCTL_SET_SPARSE` and eliding zero allocation explicitly.
    #[must_use]
    #[instrument(skip_all)]
    pub fn into_logical_read_plan(&self, cluster_size: Information) -> LogicalReadPlan {
        let mut segments = BTreeSet::default();
        let mut logical_offset = Information::ZERO;
        if cluster_size == Information::ZERO {
            return LogicalReadPlan { segments };
        }
        for run in self.iter() {
            let length_clusters = run.length_clusters;
            if length_clusters == 0 {
                continue;
            }
            #[expect(clippy::cast_possible_truncation, reason = "safe on 64-bit systems")]
            let length = length_clusters as usize * cluster_size;
            let kind = match run.local_cluster_network_start_entry_index {
                Some(lcn) => LogicalFileSegmentKind::Physical {
                    #[expect(clippy::cast_possible_truncation, reason = "safe on 64-bit systems")]
                    physical_offset: lcn as usize * cluster_size,
                },
                None => LogicalFileSegmentKind::Sparse,
            };
            segments.insert(LogicalFileSegment {
                logical_offset,
                length,
                kind,
            });
            logical_offset += length;
        }
        LogicalReadPlan { segments }
    }
}
impl Deref for MftRecordAttributeRunListOwned {
    type Target = Vec<MftRecordAttributeRunListEntry>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for MftRecordAttributeRunListOwned {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_runlist_stops_after_first_error() {
        for bytes in [&[0x10][..], &[0x12, 1][..], &[0x21, 1, 1][..]] {
            let mut runs = MftRecordAttributeRunList::new(bytes).iter();
            let _error = runs.next().unwrap().unwrap_err();
            assert!(
                runs.next().is_none(),
                "parser continued after error: {bytes:?}"
            );
        }
    }

    #[test]
    fn oversized_runlist_fields_are_rejected_without_panicking() {
        for header in [0x19, 0x91] {
            let mut bytes = vec![header];
            bytes.extend([1; 16]);
            let error = MftRecordAttributeRunList::new(&bytes)
                .iter()
                .next()
                .unwrap()
                .unwrap_err();
            assert_eq!(error.to_string(), "Run field width exceeds 8 bytes");
        }
    }

    #[test]
    fn eight_byte_negative_delta_is_valid() {
        let bytes = [
            0x11, 1, 10, 0x81, 1, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0,
        ];
        let runs = MftRecordAttributeRunList::new(&bytes).decode_all().unwrap();
        assert_eq!(runs[1].local_cluster_network_start_entry_index, Some(9));
    }

    #[test]
    fn physical_sparse_and_backward_runs_decode() {
        let runs = MftRecordAttributeRunList::new(&[0x11, 2, 10, 0x01, 3, 0x11, 1, 0xfe, 0])
            .decode_all()
            .unwrap();
        assert_eq!(
            runs.iter()
                .map(|run| run.local_cluster_network_start_entry_index)
                .collect::<Vec<_>>(),
            [Some(10), None, Some(8)]
        );
    }

    #[test]
    fn malformed_mft_map_is_not_returned_as_partial_success() {
        let runlist = [0x11, 1, 10, 0x12, 1];
        let attribute_len = 0x40 + runlist.len();
        let mut data = vec![0; 0x40 + attribute_len + 4];
        data[..4].copy_from_slice(b"FILE");
        data[0x14..0x16].copy_from_slice(&0x40u16.to_le_bytes());
        let used_size = u32::try_from(data.len()).unwrap();
        data[0x18..0x1c].copy_from_slice(&used_size.to_le_bytes());
        let attribute = &mut data[0x40..0x40 + attribute_len];
        attribute[..4].copy_from_slice(&0x80u32.to_le_bytes());
        attribute[4..8].copy_from_slice(&u32::try_from(attribute_len).unwrap().to_le_bytes());
        attribute[8] = 1;
        attribute[0x20..0x22].copy_from_slice(&0x40u16.to_le_bytes());
        attribute[0x40..].copy_from_slice(&runlist);
        data[0x40 + attribute_len..].copy_from_slice(&u32::MAX.to_le_bytes());
        let record = MftRecord::from_bytes_unchecked(bytes::Bytes::from(data));
        let error = MftRecordAttributeRunListOwned::from_mft_record(&record).unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "Failed decoding $MFT data run list: Run length field exceeds buffer"
        );
    }
}
