use chrono::DateTime;
use chrono::Local;
use eyre::Context;
use eyre::OptionExt;
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;
use super::robocopy_file_pattern::RobocopyFilePattern;
use super::robocopy_options::RobocopyOptions;

/*
-------------------------------------------------------------------------------
   ROBOCOPY     ::     Robust File Copy for Windows
-------------------------------------------------------------------------------

  Started : August 27, 2025 10:19:37 PM
   Source : J:\
     Dest : K:\

    Files : *.*

  Options : *.* /TEE /S /E /DCOPY:DA /COPY:DAT /MT:16 /R:1000000 /W:5

------------------------------------------------------------------------------
*/
pub struct RobocopyHeader {
    pub started: DateTime<Local>,
    pub source: PathBuf,
    pub dest: PathBuf,
    pub files: RobocopyFilePattern,
    pub options: RobocopyOptions,
}
impl Display for RobocopyHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "-------------------------------------------------------------------------------
   ROBOCOPY     ::     Robust File Copy for Windows                               
-------------------------------------------------------------------------------
    Started : {}
    Source : {}
        Dest : {}

    Files : {}

    Options : {}

------------------------------------------------------------------------------",
            self.started,
            self.source.display(),
            self.dest.display(),
            self.files,
            self.options
        )
    }
}
impl FromStr for RobocopyHeader {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut lines = s.lines();
        let started = lines.nth(1).ok_or_eyre("Missing started line")?;
        let source = lines.nth(1).ok_or_eyre("Missing source line")?;
        let dest = lines.nth(1).ok_or_eyre("Missing dest line")?;
        let files = lines.nth(1).ok_or_eyre("Missing files line")?;
        let options = lines.nth(1).ok_or_eyre("Missing options line")?;

        Ok(RobocopyHeader {
            started: DateTime::parse_from_rfc3339(started.trim())
                .wrap_err("Invalid started date")?
                .into(),
            source: PathBuf::from(source.trim()),
            dest: PathBuf::from(dest.trim()),
            files: RobocopyFilePattern::from_str(files.trim()).wrap_err("Invalid files pattern")?,
            options: RobocopyOptions::from_str(options.trim()).wrap_err("Invalid options")?,
        })
    }
}

// RobocopyFilePattern and RobocopyOptions moved to their own modules.
