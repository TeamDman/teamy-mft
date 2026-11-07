use arbitrary::Arbitrary;
use eyre::WrapErr;
use eyre::ensure;
use facet::Facet;
use std::fmt::Display;
use std::path::Component;
use std::path::Path;
use std::path::Prefix;
use std::str::FromStr;

/// An ASCII drive letter, stored in uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Facet)]
#[facet(opaque, proxy = String)]
#[repr(transparent)]
pub struct DriveLetter(char);

impl DriveLetter {
    pub const A: Self = Self('A');
    pub const B: Self = Self('B');
    pub const C: Self = Self('C');
    pub const D: Self = Self('D');
    pub const E: Self = Self('E');
    pub const F: Self = Self('F');
    pub const G: Self = Self('G');
    pub const H: Self = Self('H');
    pub const I: Self = Self('I');
    pub const J: Self = Self('J');
    pub const K: Self = Self('K');
    pub const L: Self = Self('L');
    pub const M: Self = Self('M');
    pub const N: Self = Self('N');
    pub const O: Self = Self('O');
    pub const P: Self = Self('P');
    pub const Q: Self = Self('Q');
    pub const R: Self = Self('R');
    pub const S: Self = Self('S');
    pub const T: Self = Self('T');
    pub const U: Self = Self('U');
    pub const V: Self = Self('V');
    pub const W: Self = Self('W');
    pub const X: Self = Self('X');
    pub const Y: Self = Self('Y');
    pub const Z: Self = Self('Z');

    pub const ALL: [Self; 26] = [
        Self::A,
        Self::B,
        Self::C,
        Self::D,
        Self::E,
        Self::F,
        Self::G,
        Self::H,
        Self::I,
        Self::J,
        Self::K,
        Self::L,
        Self::M,
        Self::N,
        Self::O,
        Self::P,
        Self::Q,
        Self::R,
        Self::S,
        Self::T,
        Self::U,
        Self::V,
        Self::W,
        Self::X,
        Self::Y,
        Self::Z,
    ];

    /// Parse the drive of an absolute Windows path without resolving it.
    ///
    /// # Errors
    ///
    /// Returns an error for relative paths, UNC paths or invalid drive letters.
    pub fn from_absolute_windows_path(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        if path.is_absolute()
            && let Some(Component::Prefix(prefix)) = path.components().next()
            && let Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) = prefix.kind()
        {
            return Self::try_from(drive);
        }
        eyre::bail!(
            "Path {} must be an absolute Windows drive path",
            path.display()
        );
    }
}

impl TryFrom<char> for DriveLetter {
    type Error = eyre::Report;

    fn try_from(value: char) -> Result<Self, Self::Error> {
        ensure!(
            value.is_ascii_alphabetic(),
            "Invalid drive letter {value:?}: expected an ASCII letter A-Z"
        );
        Ok(Self(value.to_ascii_uppercase()))
    }
}

impl TryFrom<u8> for DriveLetter {
    type Error = eyre::Report;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::try_from(char::from(value))
    }
}

impl TryFrom<String> for DriveLetter {
    type Error = eyre::Report;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl FromStr for DriveLetter {
    type Err = eyre::Report;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(
            value
                .parse::<char>()
                .wrap_err("Drive letter must contain exactly one ASCII letter")?,
        )
    }
}

impl From<DriveLetter> for char {
    fn from(value: DriveLetter) -> Self {
        value.0
    }
}

impl From<&DriveLetter> for char {
    fn from(value: &DriveLetter) -> Self {
        value.0
    }
}

impl From<DriveLetter> for u8 {
    fn from(value: DriveLetter) -> Self {
        Self::try_from(value.0).expect("DriveLetter stores only uppercase ASCII")
    }
}

impl From<DriveLetter> for String {
    fn from(value: DriveLetter) -> Self {
        value.to_string()
    }
}

impl From<&DriveLetter> for String {
    fn from(value: &DriveLetter) -> Self {
        value.to_string()
    }
}

impl Display for DriveLetter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a> Arbitrary<'a> for DriveLetter {
    fn arbitrary(input: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self(char::from(input.int_in_range(b'A'..=b'Z')?)))
    }
}

#[cfg(test)]
mod tests {
    use super::DriveLetter;

    #[cfg(windows)]
    #[test]
    fn drive_paths_parse_normalized_and_verbatim_letters() -> eyre::Result<()> {
        for (path, expected) in [
            (r"c:\repo\file.rules", DriveLetter::C),
            (r"\\?\d:\repo\file.rules", DriveLetter::D),
            (r"c:\", DriveLetter::C),
        ] {
            assert_eq!(DriveLetter::from_absolute_windows_path(path)?, expected);
        }
        for path in [
            "",
            "relative.rules",
            "C:",
            r"C:relative.rules",
            r"\repo\file.rules",
            r"\\server\share\file.rules",
            r"\\?\UNC\server\share\file.rules",
        ] {
            assert!(DriveLetter::from_absolute_windows_path(path).is_err());
        }
        Ok(())
    }

    #[test]
    fn constructors_normalize_ascii_letters() -> eyre::Result<()> {
        for (index, letter) in DriveLetter::ALL.into_iter().enumerate() {
            let uppercase = char::from(letter);
            let lowercase = uppercase.to_ascii_lowercase();
            assert_eq!(uppercase, char::from(b'A' + u8::try_from(index)?));
            assert_eq!(DriveLetter::try_from(uppercase)?, letter);
            assert_eq!(DriveLetter::try_from(lowercase)?, letter);
            assert_eq!(DriveLetter::try_from(u8::try_from(lowercase)?)?, letter);
            assert_eq!(DriveLetter::try_from(lowercase.to_string())?, letter);
            assert_eq!(lowercase.to_string().parse::<DriveLetter>()?, letter);
            assert_eq!(u8::from(letter), u8::try_from(uppercase)?);
            assert_eq!(letter.to_string(), uppercase.to_string());
        }
        Ok(())
    }

    #[test]
    fn constructors_reject_non_letters_and_multi_character_inputs() {
        for value in ['\0', ' ', ':', '/', '1', 'é', 'ß', 'Ω'] {
            assert!(DriveLetter::try_from(value).is_err());
        }
        for value in [0, b' ', b':', b'1', 255] {
            assert!(DriveLetter::try_from(value).is_err());
        }
        for value in ["", "CC", "C:", "C:\\", " C", "C ", "é", "1"] {
            assert!(value.parse::<DriveLetter>().is_err());
        }
    }

    #[test]
    fn facet_string_proxy_validates_and_normalizes_drive_letters() -> eyre::Result<()> {
        for letter in DriveLetter::ALL {
            let json = facet_json::to_string(&letter)?;
            assert_eq!(json, format!("\"{letter}\""));
            assert_eq!(facet_json::from_str::<DriveLetter>(&json)?, letter);
            assert_eq!(
                facet_json::from_str::<DriveLetter>(&json.to_ascii_lowercase())?,
                letter
            );
        }
        for json in ["\"1\"", "\"é\"", "\"CC\"", "\"\\u0000\"", "null", "34"] {
            assert!(
                facet_json::from_str::<DriveLetter>(json).is_err(),
                "invalid drive letter JSON {json} was accepted"
            );
        }
        Ok(())
    }
}
