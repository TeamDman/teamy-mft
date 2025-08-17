use crate::mft::mft_record_attribute::MftRecordAttribute;
use crate::mft::mft_record_attribute_run_list::MftRecordAttributeRunList;
use crate::mft::mft_record_attribute_run_list::MftRecordAttributeRunListEntry;
use eyre::Result;
use eyre::bail;
use eyre::eyre;
use std::ops::Deref;

/// Wrapper specific to a type 0x80 ($DATA) attribute.
/// Exposes helpers for resident / non-resident variants.
#[derive(Clone, Copy, Debug)]
pub struct MftRecordX80DollarDataAttribute<'a> {
    inner: MftRecordAttribute<'a>,
}

impl<'a> MftRecordX80DollarDataAttribute<'a> {
    pub fn new(attr: MftRecordAttribute<'a>) -> Result<Self> {
        if attr.get_attr_type() != MftRecordAttribute::TYPE_DOLLAR_DATA {
            bail!(
                "Attribute type {:X} is not DATA (0x80)",
                attr.get_attr_type()
            );
        }
        Ok(Self { inner: attr })
    }

    pub fn get_data_run_list(&'a self) -> Result<MftRecordAttributeRunList<'a>> {
        let rl = self
            .get_run_list()? // generic accessor
            .ok_or_else(|| eyre!("No data run list for resident $DATA attribute"))?;
        Ok(rl)
    }
}
impl<'a> Deref for MftRecordX80DollarDataAttribute<'a> {
    type Target = MftRecordAttribute<'a>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Backwards compatibility alias (temporary) for code still using `DataRunEntry`.
pub type DataRunEntry = MftRecordAttributeRunListEntry;
