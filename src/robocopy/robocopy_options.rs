use std::fmt::Display;
use std::str::FromStr;

#[derive(Debug, PartialEq, Eq)]
pub struct RobocopyOptions {
    pub(crate) inner: String,
}
impl Display for RobocopyOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}
impl FromStr for RobocopyOptions {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(RobocopyOptions {
            inner: s.to_string(),
        })
    }
}
