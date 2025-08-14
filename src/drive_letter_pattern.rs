use arbitrary::Arbitrary;
use color_eyre::eyre::{self as eyre};
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
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Resolve the pattern into a list of drive letters.
    pub fn into_drive_letters(&self) -> eyre::Result<Vec<char>> {
        parse_drive_letters(self.as_str())
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
        if s.is_empty() {
            return Err(eyre::eyre!("empty drive letter pattern"));
        }
        Ok(DriveLetterPattern(s.to_string()))
    }
}

impl<'a> Arbitrary<'a> for DriveLetterPattern {
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

/// Parse drive letters from input string, handling wildcards and multiple drives
fn parse_drive_letters(input: &str) -> eyre::Result<Vec<char>> {
    let input = input.trim();

    if input == "*" {
        // Get all available drives
        return get_available_drives();
    }

    // Parse individual drive letters
    let mut drives = Vec::new();

    // Handle various separators: space, comma, semicolon
    let parts: Vec<&str> = input
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|s| !s.is_empty())
        .collect();

    for part in parts {
        let part = part.trim();
        if part.len() == 1 {
            if let Some(drive_char) = part.chars().next() {
                if drive_char.is_ascii_alphabetic() {
                    drives.push(drive_char.to_ascii_uppercase());
                } else {
                    return Err(eyre::eyre!("Invalid drive letter: '{}'", part));
                }
            }
        } else if part.len() > 1 {
            // Handle multiple characters as individual drive letters
            for drive_char in part.chars() {
                if drive_char.is_ascii_alphabetic() {
                    drives.push(drive_char.to_ascii_uppercase());
                } else {
                    return Err(eyre::eyre!("Invalid drive letter: '{}'", drive_char));
                }
            }
        }
    }

    if drives.is_empty() {
        return Err(eyre::eyre!("No valid drive letters found in: '{}'", input));
    }

    Ok(drives)
}

/// Get all available drives on the system
fn get_available_drives() -> eyre::Result<Vec<char>> {
    use windows::Win32::Storage::FileSystem::GetLogicalDrives;

    let drives_bitmask = unsafe { GetLogicalDrives() };

    let mut available_drives = Vec::new();
    for i in 0..26 {
        if (drives_bitmask & (1 << i)) != 0 {
            available_drives.push((b'A' + i as u8) as char);
        }
    }

    if available_drives.is_empty() {
        return Err(eyre::eyre!("No drives found on system"));
    }

    Ok(available_drives)
}
