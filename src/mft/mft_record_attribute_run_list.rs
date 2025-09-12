use crate::mft::mft_record::MftRecord;
use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::logical_read_plan::LogicalReadSegment;
use crate::read::logical_read_plan::LogicalReadSegmentKind;
use crate::read::physical_read_plan::PhysicalReadPlan;
use eyre::Result;
use eyre::eyre;
use std::ops::Deref;
use std::ops::DerefMut;
use tracing::warn;
use uom::ConstZero;
use uom::si::information::byte;
use uom::si::u64::Information;

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

    /// Convert decoded run list entries into a simple list of physical (byte) read requests.
    ///
    /// Responsibilities (phase 1 of pipeline):
    ///  - Skip sparse runs (logical gaps -> zero fill handled later if needed).
    ///  - Convert cluster-based extents into byte offsets using bytes_per_cluster.
    ///  - Coalesce immediately contiguous physical extents (where next_lcn == prev_lcn + prev_len_clusters)
    ///
    /// NOT handled here:
    ///  - Splitting into IO-sized chunks (that is a later pipeline stage).
    ///  - Performing any actual I/O.
    ///  - Injecting zero buffers for sparse gaps.
    pub fn into_physical_reader(&self, bytes_per_cluster: u64) -> PhysicalReadPlan {
        // NOTE: This legacy helper now returns an unchunked PhysicalReadPlan.
        let mut plan = PhysicalReadPlan::new();
        if bytes_per_cluster == 0 {
            return plan;
        }
        let mut logical_offset: u64 = 0;
        for run in self.iter() {
            let Some(lcn) = run.local_cluster_network_start_entry_index else {
                continue;
            }; // skip sparse
            if run.length_clusters == 0 {
                continue;
            }
            let Some(phys) = lcn.checked_mul(bytes_per_cluster) else {
                warn!("LCN * bytes_per_cluster overflow; skipping run");
                continue;
            };
            let Some(len_bytes) = run.length_clusters.checked_mul(bytes_per_cluster) else {
                warn!("Run length * bytes_per_cluster overflow; skipping run");
                continue;
            };
            plan.push(
                Information::new::<byte>(phys),
                Information::new::<byte>(logical_offset),
                Information::new::<byte>(len_bytes),
            );
            logical_offset = logical_offset.saturating_add(len_bytes);
        }
        plan.merge_contiguous_reads();
        plan
    }
}


impl MftRecordAttributeRunListOwned {
    /// Build a logical plan preserving sparse runs. Future opportunity (Option C): produce a sparse output file
    /// by marking destination with FSCTL_SET_SPARSE and eliding zero allocation explicitly.
    pub fn into_logical_read_plan(&self, cluster_size: Information) -> LogicalReadPlan {
        let mut segments = Vec::new();
        let mut logical_offset = Information::ZERO;
        if cluster_size == Information::ZERO {
            return LogicalReadPlan {
                segments,
                total_logical_size_bytes: 0,
            };
        }
        for run in self.iter() {
            let length_clusters = run.length_clusters;
            if length_clusters == 0 {
                continue;
            }
            let length_bytes = length_clusters * cluster_size;
            let kind = match run.local_cluster_network_start_entry_index {
                Some(lcn) => LogicalReadSegmentKind::Physical {
                    physical_offset_bytes: (lcn * cluster_size).get::<byte>(),
                },
                None => LogicalReadSegmentKind::Sparse,
            };
            segments.push(LogicalReadSegment {
                logical_offset_bytes: logical_offset.get::<byte>(),
                length_bytes: length_bytes.get::<byte>(),
                kind,
            });
            logical_offset += length_bytes;
        }
        LogicalReadPlan {
            segments,
            total_logical_size_bytes: logical_offset.get::<byte>(),
        }
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
