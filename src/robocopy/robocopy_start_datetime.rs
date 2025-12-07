use chrono::DateTime;
use chrono::Local;
use chrono::LocalResult;
use chrono::NaiveDateTime;
use chrono::TimeZone;
use std::fmt::Display;
use std::ops::Deref;
use std::str::FromStr;

#[derive(Debug, PartialEq, Eq)]
pub struct RobocopyStartDateTime {
    inner: DateTime<Local>,
}
impl Display for RobocopyStartDateTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Robocopy start date display format
        write!(f, "{}", self.inner.format(ROBOCOPY_START_DATETIME_FMT))
    }
}
const ROBOCOPY_START_DATETIME_FMT: &str = "%B %d, %Y %I:%M:%S %p"; // e.g. August 27, 2025 10:19:37 PM

impl FromStr for RobocopyStartDateTime {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let naive = NaiveDateTime::parse_from_str(s.trim(), ROBOCOPY_START_DATETIME_FMT)?;
        // Handle potential ambiguity (DST transitions) by picking earliest occurrence.
        let local_dt = match Local.from_local_datetime(&naive) {
            LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => dt, // choose the earlier
            LocalResult::None => eyre::bail!("Invalid local datetime (non-existent due to DST)"),
        };
        Ok(Self { inner: local_dt })
    }
}

impl RobocopyStartDateTime {
    #[must_use]
    pub fn as_datetime(&self) -> &DateTime<Local> {
        &self.inner
    }
}
impl From<DateTime<Local>> for RobocopyStartDateTime {
    fn from(dt: DateTime<Local>) -> Self {
        Self { inner: dt }
    }
}
impl Deref for RobocopyStartDateTime {
    type Target = DateTime<Local>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(test)]
mod test {
    use super::RobocopyStartDateTime;
    use chrono::Local;
    use chrono::TimeZone;

    #[test]
    fn parse_date() -> eyre::Result<()> {
        let s = "August 27, 2025 10:19:37 PM";
        let parsed: RobocopyStartDateTime = s.parse()?;
        let expected = Local.with_ymd_and_hms(2025, 8, 27, 22, 19, 37).unwrap();
        assert_eq!(*parsed.as_datetime(), expected);
        assert_eq!(parsed.to_string(), s);
        Ok(())
    }
}
