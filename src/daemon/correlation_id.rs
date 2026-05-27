use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, vox::facet::Facet)]
#[facet(opaque, proxy = String)]
pub struct CorrelationId(pub Uuid);

impl CorrelationId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for CorrelationId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<&CorrelationId> for String {
    fn from(value: &CorrelationId) -> Self {
        value.to_string()
    }
}

impl TryFrom<String> for CorrelationId {
    type Error = uuid::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl std::fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for CorrelationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

// SAFETY: `CorrelationId` is an owned UUID value with no borrowed fields, so its
// reborrowed representation can be the same owned wire shape.
unsafe impl vox_types::Reborrow for CorrelationId {
    type Ref<'a> = CorrelationId;
}
