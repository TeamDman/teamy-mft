use crate::mft::mft_record_attribute_non_resident_header::MftRecordAttributeNonResidentHeader;
use crate::mft::mft_record_attribute_run_list::MftRecordAttributeRunList;
use crate::mft::mft_record_attribute_x80_data_attribute::MftRecordX80DollarDataAttribute;
use eyre::bail;
use eyre::eyre;
use std::ops::Deref;

/// Wrapper around a borrowed attribute slice inside an MFT record.
/// Provides typed accessors for common header fields without copying.
#[derive(Clone, Copy, Debug)]
pub struct MftRecordAttribute<'a> {
    mft_record_attribute_data: &'a [u8],
}

impl<'a> MftRecordAttribute<'a> {
    pub const TYPE_END: u32 = 0xFFFF_FFFF;
    pub const TYPE_DOLLAR_DATA: u32 = 0x80;

    /// # Errors
    ///
    /// Returns an error if the attribute data is too short.
    pub fn new(mft_record_attribute_data: &'a [u8]) -> eyre::Result<Self> {
        if mft_record_attribute_data.len() < 16 {
            bail!(
                "Attribute slice too short ({} bytes)",
                mft_record_attribute_data.len()
            );
        }
        Ok(Self {
            mft_record_attribute_data,
        })
    }
    #[inline]
    #[must_use]
    /// # Panics
    ///
    /// Panics if the attribute data is too short.
    pub fn get_attr_type(&self) -> u32 {
        u32::from_le_bytes(self.mft_record_attribute_data[0..4].try_into().unwrap())
    }

    #[inline]
    #[must_use]
    /// # Panics
    ///
    /// Panics if the attribute data is too short.
    pub fn get_total_length(&self) -> u32 {
        u32::from_le_bytes(self.mft_record_attribute_data[4..8].try_into().unwrap())
    }

    #[inline]
    #[must_use]
    pub fn get_is_non_resident(&self) -> bool {
        self.mft_record_attribute_data[8] != 0
    }

    #[inline]
    #[must_use]
    pub fn get_name_len(&self) -> u8 {
        self.mft_record_attribute_data[9]
    }

    #[inline]
    #[must_use]
    /// # Panics
    ///
    /// Panics if the attribute data is too short.
    pub fn get_name_offset(&self) -> u16 {
        u16::from_le_bytes(self.mft_record_attribute_data[10..12].try_into().unwrap())
    }

    #[inline]
    #[must_use]
    /// # Panics
    ///
    /// Panics if the attribute data is too short.
    pub fn get_flags(&self) -> u16 {
        u16::from_le_bytes(self.mft_record_attribute_data[12..14].try_into().unwrap())
    }

    #[inline]
    #[must_use]
    /// # Panics
    ///
    /// Panics if the attribute data is too short.
    pub fn get_attr_id(&self) -> u16 {
        u16::from_le_bytes(self.mft_record_attribute_data[14..16].try_into().unwrap())
    }

    // Resident specific
    #[must_use]
    pub fn get_resident_content(&self) -> Option<&[u8]> {
        if self.get_is_non_resident() || self.mft_record_attribute_data.len() < 0x18 {
            return None;
        }
        let size = u32::from_le_bytes(self.mft_record_attribute_data[0x10..0x14].try_into().ok()?)
            as usize;
        let off = u16::from_le_bytes(self.mft_record_attribute_data[0x14..0x16].try_into().ok()?)
            as usize;
        if off + size > self.mft_record_attribute_data.len() {
            return None;
        }
        Some(&self.mft_record_attribute_data[off..off + size])
    }

    // Non-resident specific
    #[must_use]
    pub fn get_non_resident_header(&self) -> Option<MftRecordAttributeNonResidentHeader<'_>> {
        if !self.get_is_non_resident() || self.mft_record_attribute_data.len() < 0x40 {
            return None;
        }
        Some(MftRecordAttributeNonResidentHeader::new(self))
    }

    #[must_use]
    pub fn as_x80(&self) -> Option<MftRecordX80DollarDataAttribute<'_>> {
        MftRecordX80DollarDataAttribute::new(*self).ok()
    }

    #[inline]
    pub(crate) fn raw_data(&self) -> &'a [u8] {
        self.mft_record_attribute_data
    }

    /// For any non-resident attribute returns a `RunList` starting at the runlist offset.
    /// Returns Ok(None) for resident attributes. Errors if the encoded offset is out of bounds.
    ///
    /// # Errors
    ///
    /// Returns an error if the attribute is non-resident but the header or runlist is invalid.
    pub fn get_run_list(&self) -> eyre::Result<Option<MftRecordAttributeRunList<'_>>> {
        if !self.get_is_non_resident() {
            return Ok(None);
        }
        let header = self
            .get_non_resident_header()
            .ok_or_else(|| eyre!("Attribute marked non-resident but too short for header"))?;
        Ok(Some(MftRecordAttributeRunList::new(header.runlist()?)))
    }
}
impl Deref for MftRecordAttribute<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.mft_record_attribute_data
    }
}
