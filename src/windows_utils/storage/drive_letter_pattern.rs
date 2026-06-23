use crate::windows_utils::string::EasyPCWSTR;
use arbitrary::Arbitrary;
use eyre::ensure;
use facet::Facet;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use windows::Win32::System::WindowsProgramming::DRIVE_REMOTE;

/// Represents a user-provided drive letter pattern.
/// Examples:
/// - "*" -> all drives
/// - "C" -> just C
/// - "CD" -> C and D
/// - "C,D;E F" -> C, D, E, F (separators: space/comma/semicolon)
#[derive(Clone, PartialEq, Eq, Debug, Facet, Arbitrary)]
#[facet(transparent)]
pub struct DriveLetterPattern(pub String);
impl Default for DriveLetterPattern {
    fn default() -> Self {
        DriveLetterPattern("*".to_string())
    }
}

impl DriveLetterPattern {
    /// Resolve the pattern into a list of drive letters.
    ///
    /// # Errors
    ///
    /// Returns an error if the pattern is invalid or no drives are found.
    pub fn into_drive_letters(&self) -> eyre::Result<Vec<char>> {
        let input = self.as_ref().trim();

        if input == "*" {
            return get_available_drives();
        }

        let mut rtn = Vec::new();

        for (i, char) in input.chars().enumerate() {
            let skippable = char.is_whitespace() || char == ',' || char == ';';
            if skippable {
                continue;
            }

            ensure!(
                char.is_ascii_alphabetic(),
                "Invalid drive letter character at position {i}: '{char}'"
            );

            rtn.push(char.to_ascii_uppercase());
        }

        ensure!(!rtn.is_empty(), "No drive letters found in: '{}'", input);

        Ok(rtn)
    }

    /// Resolve the pattern, inferring drives from scope roots when the pattern
    /// is the wildcard default.
    ///
    /// # Errors
    ///
    /// Returns an error if the explicit pattern is invalid, no drives can be
    /// inferred from scope roots, or no drives are available.
    pub fn into_drive_letters_for_scope_roots<'a>(
        &self,
        scope_roots: impl IntoIterator<Item = &'a Path>,
    ) -> eyre::Result<Vec<char>> {
        if self != &Self::default() {
            return self.into_drive_letters();
        }

        let mut drive_letters = Vec::new();
        for scope_root in scope_roots {
            let Some(prefix) = scope_root.components().next() else {
                continue;
            };
            let std::path::Component::Prefix(prefix) = prefix else {
                continue;
            };
            let (std::path::Prefix::Disk(drive) | std::path::Prefix::VerbatimDisk(drive)) =
                prefix.kind()
            else {
                continue;
            };
            let drive = char::from(drive).to_ascii_uppercase();
            if !drive_letters.contains(&drive) {
                drive_letters.push(drive);
            }
        }

        if drive_letters.is_empty() {
            self.into_drive_letters()
        } else {
            Ok(drive_letters)
        }
    }
}

impl fmt::Display for DriveLetterPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for DriveLetterPattern {
    type Err = eyre::Report;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        ensure!(!s.is_empty(), "empty drive letter pattern");
        Ok(DriveLetterPattern(s.to_string()))
    }
}
impl AsRef<str> for DriveLetterPattern {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Get all available drives on the system
///
/// Maybe see also:
/// <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getlogicaldrivestringsw>
/// <https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file>
fn get_available_drives() -> eyre::Result<Vec<char>> {
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows::Win32::Storage::FileSystem::GetLogicalDrives;

    // SAFETY: GetLogicalDrives is a safe Windows API call that returns a bitmask of available drives.
    let drives_bitmask = unsafe { GetLogicalDrives() };

    let mut available_drives = Vec::new();
    for i in 0..26 {
        if (drives_bitmask & (1 << i)) != 0 {
            // i is constrained 0..26, convert explicitly to u8 to avoid truncation warnings
            let idx = u8::try_from(i).unwrap_or_default();
            let drive_letter = (b'A' + idx) as char;
            let drive_root = format!("{drive_letter}:\\");
            let drive_root = drive_root.easy_pcwstr()?;

            // SAFETY: the root path is a valid, null-terminated UTF-16 string for the duration of the call.
            let drive_type = unsafe { GetDriveTypeW(drive_root.as_ref()) };
            if should_enumerate_drive_type(drive_type) {
                available_drives.push(drive_letter);
            }
        }
    }

    ensure!(!available_drives.is_empty(), "No drives found on system");

    Ok(available_drives)
}

fn should_enumerate_drive_type(drive_type: u32) -> bool {
    drive_type != DRIVE_REMOTE
}

#[cfg(test)]
mod tests {
    use super::DRIVE_REMOTE;
    use super::DriveLetterPattern;
    use super::should_enumerate_drive_type;

    #[test]
    fn parses_explicit_drive_letters_with_separators() -> eyre::Result<()> {
        let drive_letters = DriveLetterPattern("C,D;E F".to_string()).into_drive_letters()?;
        assert_eq!(drive_letters, vec!['C', 'D', 'E', 'F']);
        Ok(())
    }

    #[test]
    fn rejects_non_alphabetic_drive_letters() {
        let error = DriveLetterPattern("C1".to_string())
            .into_drive_letters()
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid drive letter character at position 1: '1'")
        );
    }

    #[test]
    fn default_pattern_infers_drives_from_scope_roots() -> eyre::Result<()> {
        let drive_letters = DriveLetterPattern::default().into_drive_letters_for_scope_roots([
            std::path::Path::new(r"G:\repo"),
            std::path::Path::new(r"G:\other"),
            std::path::Path::new(r"C:\repo"),
        ])?;

        assert_eq!(drive_letters, vec!['G', 'C']);
        Ok(())
    }

    #[test]
    fn explicit_pattern_overrides_scope_roots() -> eyre::Result<()> {
        let drive_letters = DriveLetterPattern("C".to_string())
            .into_drive_letters_for_scope_roots([std::path::Path::new(r"G:\repo")])?;

        assert_eq!(drive_letters, vec!['C']);
        Ok(())
    }

    #[test]
    fn excludes_remote_drive_types_from_wildcard_enumeration() {
        assert!(!should_enumerate_drive_type(DRIVE_REMOTE));
        assert!(should_enumerate_drive_type(3));
    }
}
