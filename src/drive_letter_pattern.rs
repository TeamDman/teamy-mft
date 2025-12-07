use arbitrary::Arbitrary;
use serde::Deserialize;
use serde::Serialize;
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

impl<'de> Deserialize<'de> for DriveLetterPattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl Serialize for DriveLetterPattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}
// tests are placed at the end of the file

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
            // i is constrained 0..26, convert explicitly to u8 to avoid truncation warnings
            let idx = u8::try_from(i).unwrap_or_default();
            available_drives.push((b'A' + idx) as char);
        }
    }

    if available_drives.is_empty() {
        return Err(eyre::eyre!("No drives found on system"));
    }

    Ok(available_drives)
}

#[cfg(test)]
mod test {
    use crate::drive_letter_pattern::DriveLetterPattern;

    #[tokio::test]
    async fn serialize() -> eyre::Result<()> {
        let pattern = DriveLetterPattern("C,D;E F".to_string());
        let serialized = serde_json::to_string(&pattern)?;
        assert_eq!(serialized, "\"C,D;E F\"");
        Ok(())
    }
    #[tokio::test]
    async fn deserialize() -> eyre::Result<()> {
        let s = "\"C,D;E F\"";
        let deserialized: DriveLetterPattern = serde_json::from_str(s)?;
        assert_eq!(deserialized, DriveLetterPattern("C,D;E F".to_string()));
        Ok(())
    }
}
