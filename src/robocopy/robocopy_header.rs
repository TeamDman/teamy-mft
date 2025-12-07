use super::robocopy_file_pattern::RobocopyFilePattern;
use super::robocopy_options::RobocopyOptions;
use super::robocopy_start_datetime::RobocopyStartDateTime;
use chrono::DateTime;
use chrono::Local;
use eyre::OptionExt;
use eyre::WrapErr;
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

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
#[derive(Debug, PartialEq, Eq)]
pub struct RobocopyHeader {
    pub started: RobocopyStartDateTime,
    pub source: PathBuf,
    pub dest: PathBuf,
    pub files: RobocopyFilePattern,
    pub options: RobocopyOptions,
}
impl Display for RobocopyHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "-------------------------------------------------------------------------------\n   ROBOCOPY     ::     Robust File Copy for Windows                              \n-------------------------------------------------------------------------------\n\n  Started : {started}\n   Source : {source}\n     Dest : {dest}\n\n    Files : {files}\n\t    \n  Options : {options} \n\n------------------------------------------------------------------------------",
            started = self.started,
            source = self.source.display(),
            dest = self.dest.display(),
            files = self.files,
            options = self.options
        )
    }
}
impl FromStr for RobocopyHeader {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut started: Option<DateTime<Local>> = None;
        let mut source: Option<PathBuf> = None;
        let mut dest: Option<PathBuf> = None;
        let mut files: Option<RobocopyFilePattern> = None;
        let mut options: Option<RobocopyOptions> = None;

        for raw_line in s.lines() {
            let line = raw_line.trim_start();
            // Skip banner / separators / blank
            if line.is_empty() || line.chars().all(|c| c == '-') || line.starts_with("ROBOCOPY") {
                continue;
            }
            // Expect KEY : VALUE format (note variable leading spaces)
            if let Some(idx) = line.find(':') {
                let (key_part, value_part) = line.split_at(idx);
                let key = key_part.trim();
                // skip ':'
                let value = value_part[1..].trim();
                match key.to_ascii_lowercase().as_str() {
                    "started" => {
                        if started.is_none() {
                            let dt: RobocopyStartDateTime =
                                value.parse().wrap_err("Invalid Started field")?;
                            started = Some(*dt.as_datetime());
                        }
                    }
                    "source" => {
                        if source.is_none() {
                            source = Some(PathBuf::from(value));
                        }
                    }
                    "dest" => {
                        if dest.is_none() {
                            dest = Some(PathBuf::from(value));
                        }
                    }
                    "files" => {
                        if files.is_none() {
                            files = Some(value.parse().wrap_err("Invalid Files pattern")?);
                        }
                    }
                    "options" => {
                        if options.is_none() {
                            options = Some(value.trim().parse().wrap_err("Invalid Options")?);
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(RobocopyHeader {
            started: started.ok_or_eyre("Missing Started field")?.into(),
            source: source.ok_or_eyre("Missing Source field")?,
            dest: dest.ok_or_eyre("Missing Dest field")?,
            files: files.ok_or_eyre("Missing Files field")?,
            options: options.ok_or_eyre("Missing Options field")?,
        })
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use chrono::TimeZone;

    fn normalize(s: &str) -> String {
        s.trim_start_matches('\n')
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parse_header_from_sample() -> eyre::Result<()> {
        // Load full sample log
        let content = include_str!("sample.txt");
        // Split on dashed lines; count occurrences and take everything until second dashed line group ends.
        let mut collected = Vec::new();
        let mut dash_count = 0;
        for line in content.lines() {
            collected.push(line);
            let trimmed = line.trim();
            if !trimmed.is_empty() && trimmed.chars().all(|c| c == '-') {
                dash_count += 1;
                if dash_count == 3 {
                    break;
                } // include full header (three separators)
            }
        }
        let header_str = collected.join("\n");
        let header: RobocopyHeader = header_str.parse()?;

        let expected_started = chrono::Local
            .with_ymd_and_hms(2025, 8, 27, 22, 19, 37)
            .unwrap();
        assert_eq!(*header.started, expected_started);
        assert_eq!(header.source, PathBuf::from("J:/"));
        assert_eq!(header.dest, PathBuf::from("K:/"));
        assert_eq!(header.files.to_string(), "*.*");
        assert_eq!(
            header.options.to_string(),
            "*.* /TEE /S /E /DCOPY:DA /COPY:DAT /MT:16 /R:1000000 /W:5"
        );
        // Normalize both expected header block and display (trim leading blank lines & trailing whitespace per line)
        let expected_display = normalize(&header_str);
        let actual_display = normalize(&header.to_string());
        assert_eq!(
            actual_display, expected_display,
            "Display formatting mismatch"
        );
        Ok(())
    }
}
