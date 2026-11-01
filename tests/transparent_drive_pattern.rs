use facet::Facet;
use teamy_mft::windows_utils::storage::DriveLetterPattern;

#[derive(Debug, Facet, PartialEq)]
struct Args {
    #[facet(figue::positional)]
    drives: DriveLetterPattern,
    #[facet(figue::named)]
    optional: Option<DriveLetterPattern>,
    #[facet(figue::named, default)]
    repeated: Vec<DriveLetterPattern>,
}

#[test]
fn drive_patterns_roundtrip_through_optional_and_repeated_cli_arguments() {
    let expected = Args {
        drives: DriveLetterPattern("C,D;E F".to_owned()),
        optional: Some(DriveLetterPattern(" c ".to_owned())),
        repeated: vec![
            DriveLetterPattern(String::new()),
            DriveLetterPattern("D".to_owned()),
        ],
    };
    let words = figue::to_os_args(&expected).unwrap();
    let words: Vec<_> = words.iter().map(|word| word.to_str().unwrap()).collect();
    let actual: Args = figue::from_slice(&words)
        .into_result()
        .unwrap()
        .get_silent();
    assert_eq!(actual, expected);
    assert_eq!(
        actual.drives.into_drive_letters().unwrap(),
        vec!['C', 'D', 'E', 'F']
    );
}

#[test]
fn drive_pattern_validation_stays_at_drive_resolution() {
    for raw in ["", "C1", "--invalid"] {
        let actual: Args = figue::from_slice(&["--", raw])
            .into_result()
            .unwrap()
            .get_silent();
        assert_eq!(actual.drives.0, raw);
        let _ = actual
            .drives
            .into_drive_letters()
            .expect_err("invalid drive pattern must be rejected");
    }
}

#[test]
fn drive_patterns_keep_required_and_optional_argument_semantics() {
    let _ = figue::from_slice::<Args>(&[])
        .into_result()
        .expect_err("drive argument is required");
    let actual: Args = figue::from_slice(&["C"])
        .into_result()
        .unwrap()
        .get_silent();
    assert_eq!(actual.drives.0, "C");
    assert_eq!(actual.optional, None);
    assert!(actual.repeated.is_empty());
}
