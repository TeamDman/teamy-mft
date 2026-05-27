use arbitrary::Arbitrary;
use facet::Facet;

#[derive(Default, Facet, Arbitrary, Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[repr(u8)]
#[strum(serialize_all = "kebab-case")]
#[facet(rename_all = "kebab-case")]
pub enum QuerySource {
    #[default]
    Auto,
    DaemonOnly,
    DiskOnly,
}
