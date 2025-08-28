use std::fmt::Display;
use std::str::FromStr;

#[derive(Debug, PartialEq, Eq)]
pub struct RobocopyFilePattern {
    pub(crate) inner: String,
}
impl Display for RobocopyFilePattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}
impl FromStr for RobocopyFilePattern {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(RobocopyFilePattern {
            inner: s.to_string(),
        })
    }
}
