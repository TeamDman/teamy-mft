#[derive(Debug, Clone, Copy, PartialEq, Eq, vox::facet::Facet)]
#[repr(u8)]
pub enum MachineErrorKind {
    Unavailable,
    Degraded,
    RequestInvalid,
}

#[derive(Debug, Clone, PartialEq, Eq, vox::facet::Facet)]
pub struct MachineError {
    pub kind: MachineErrorKind,
    pub message: String,
}

impl std::fmt::Display for MachineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MachineError {}

impl MachineError {
    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: MachineErrorKind::Unavailable,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn degraded(message: impl Into<String>) -> Self {
        Self {
            kind: MachineErrorKind::Degraded,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn request_invalid(message: impl Into<String>) -> Self {
        Self {
            kind: MachineErrorKind::RequestInvalid,
            message: message.into(),
        }
    }
}
