use arbitrary::Arbitrary;
use eyre::ensure;
use std::fmt;
use std::str::FromStr;

/// Represents a user-provided drive letter pattern.
/// Examples:
/// - "*" -> all drives
/// - "C" -> just C
/// - "CD" -> C and D
/// - "C,D;E F" -> C, D, E, F (separators: space/comma/semicolon)
#[derive(Clone, PartialEq, Eq, Debug)]
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

impl Arbitrary<'_> for DriveLetterPattern {
    fn arbitrary(u: &mut arbitrary::Unstructured<'_>) -> arbitrary::Result<Self> {
        // 20% chance wildcard, 80% chance letters
        if (u8::arbitrary(u)?).is_multiple_of(5) {
            return Ok(DriveLetterPattern("*".to_string()));
        }
        // Build between 1 and 4 letters
        let count = (u8::arbitrary(u)? % 4) + 1; // 1..=4
        let mut s = String::new();
        for i in 0..count {
            let idx = u8::arbitrary(u)? % 26;
            let c = (b'A' + idx) as char;
            if i > 0 {
                // random separator choice
                match u8::arbitrary(u)? % 3 {
                    0 => s.push(','),
                    1 => s.push(';'),
                    _ => s.push(' '),
                }
            }
            s.push(c);
        }
        Ok(DriveLetterPattern(s))
    }
}

/// Get all available drives on the system
///
/// Maybe see also:
/// <https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getlogicaldrivestringsw>
/// <https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file>
fn get_available_drives() -> eyre::Result<Vec<char>> {
    use windows::Win32::Storage::FileSystem::GetLogicalDrives;

    // SAFETY: GetLogicalDrives is a safe Windows API call that returns a bitmask of available drives.
    let drives_bitmask = unsafe { GetLogicalDrives() };

    let mut available_drives = Vec::new();
    for i in 0..26 {
        if (drives_bitmask & (1 << i)) != 0 {
            // i is constrained 0..26, convert explicitly to u8 to avoid truncation warnings
            let idx = u8::try_from(i).unwrap_or_default();
            available_drives.push((b'A' + idx) as char);
        }
    }

    ensure!(!available_drives.is_empty(), "No drives found on system");

    Ok(available_drives)
}
