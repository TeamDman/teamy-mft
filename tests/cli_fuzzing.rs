//! CLI fuzzing tests using figue's arbitrary helper assertions.

use figue::from_slice;
use teamy_mft::cli::Cli;

#[test]
// cli[verify parser.args-consistent]
fn fuzz_cli_args_consistency() {
    if let Err(e) =
        figue::assert_to_args_consistency::<Cli>(figue::TestToArgsConsistencyConfig::default())
    {
        panic!("CLI argument consistency check failed:\n{e}")
    };
}

#[test]
// cli[verify parser.roundtrip]
fn fuzz_cli_args_roundtrip() {
    if let Err(e) = figue::assert_to_args_roundtrip::<Cli>(figue::TestToArgsRoundTrip::default()) {
        panic!("CLI argument roundtrip check failed:\n{e}")
    };
}

#[test]
// tool[verify cli.help.describes-environment]
fn help_mentions_environment_variable() {
    let result = from_slice::<Cli>(&["--help"]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_help());

    let help = err.help_text().expect("help text should be present");
    assert!(
        help.contains("TEAMY_MFT_SYNC_DIR"),
        "help should mention TEAMY_MFT_SYNC_DIR, got:\n{help}"
    );
}
