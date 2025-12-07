use crate::mft::mft_record::MftRecord;
use crate::read::logical_read_plan::LogicalFileSegment;
use crate::read::logical_read_plan::LogicalFileSegmentKind;
use crate::read::logical_read_plan::LogicalReadPlan;
use eyre::Result;
use eyre::eyre;
use std::ops::Deref;
use std::ops::DerefMut;
use tracing::warn;
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
    pub fn new(raw: &'a [u8]) -> Self {
        Self { raw }
    }
    pub fn as_slice(&self) -> &'a [u8] {
        self.raw
    }
    pub fn iter(&self) -> MftRecordAttributeRunListIter<'a> {
        MftRecordAttributeRunListIter {
            raw: self.raw,
            pos: 0,
            last_lcn: 0,
        }
    }
    pub fn decode_all(&self) -> Result<Vec<MftRecordAttributeRunListEntry>> {
        self.iter().collect()
    }
}

#[derive(Debug)]
pub struct MftRecordAttributeRunListIter<'a> {
    raw: &'a [u8],
    pos: usize,
    last_lcn: i64,
}

impl<'a> Iterator for MftRecordAttributeRunListIter<'a> {
    type Item = Result<MftRecordAttributeRunListEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.raw.len() {
            return None;
        }
        let header = self.raw[self.pos];
        if header == 0 {
            return None;
        }
        let offset_size = (header & 0xF0) >> 4;
        let length_size = header & 0x0F;
        if length_size == 0 {
            return Some(Err(eyre!("Zero length_size in run header")));
        }
        self.pos += 1;
        if self.pos + length_size as usize > self.raw.len() {
            return Some(Err(eyre!("Run length field exceeds buffer")));
        }
        let mut length = 0u64;
        for i in 0..length_size {
            length |= (self.raw[self.pos + i as usize] as u64) << (8 * i);
        }
        self.pos += length_size as usize;
        let local_cluster_network_start_entry_index = if offset_size == 0 {
            None
        } else {
            if self.pos + offset_size as usize > self.raw.len() {
                return Some(Err(eyre!("Run offset field exceeds buffer")));
            }
            let mut delta: i64 = 0;
            for i in 0..offset_size {
                delta |= (self.raw[self.pos + i as usize] as i64) << (8 * i);
            }
            let sign_bit = 1i64 << (offset_size * 8 - 1);
            if delta & sign_bit != 0 {
                let mask = (!0i64) << (offset_size * 8);
                delta |= mask;
            }
            self.pos += offset_size as usize;
            self.last_lcn = self.last_lcn.wrapping_add(delta);
            Some(self.last_lcn as u64)
        };
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
    pub fn from_mft_record(dollar_mft_record: &MftRecord) -> Self {
        let mut rtn = Self::default();
        for attr in dollar_mft_record.iter_attributes() {
            if let Some(x80) = attr.as_x80() {
                // todo: ensure that the attribute has no name
                if let Ok(runlist) = x80.get_data_run_list() {
                    for run_res in runlist.iter() {
                        match run_res {
                            Ok(run) => rtn.push(run),
                            Err(e) => warn!("Failed decoding data run entry: {e:?}"),
                        }
                    }
                } else {
                    warn!("Failed to get data runs from attribute");
                }
            }
        }
        rtn
    }
}

impl MftRecordAttributeRunListOwned {
    /// Build a logical plan preserving sparse runs.
    ///
    /// Future opportunity: produce a sparse output file
    /// by marking destination with FSCTL_SET_SPARSE and eliding zero allocation explicitly.
    pub fn into_logical_read_plan(&self, cluster_size: Information) -> LogicalReadPlan {
        let mut segments = Default::default();
        let mut logical_offset = Information::ZERO;
        if cluster_size == Information::ZERO {
            return LogicalReadPlan { segments };
        }
        for run in self.iter() {
            let length_clusters = run.length_clusters;
            if length_clusters == 0 {
                continue;
            }
            let length = length_clusters as usize * cluster_size;
            let kind = match run.local_cluster_network_start_entry_index {
                Some(lcn) => LogicalFileSegmentKind::Physical {
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
