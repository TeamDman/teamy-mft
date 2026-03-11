//! CLI fuzzing tests using figue's arbitrary helper assertions.

use teamy_mft::cli::Cli;

#[test]
fn fuzz_cli_args_consistency() {
    if let Err(e) = figue::assert_to_args_consistency::<Cli>(5000) {
        panic!("CLI argument consistency check failed:\n{e}")
    };
}

#[test]
fn fuzz_cli_args_roundtrip() {
    if let Err(e) = figue::assert_to_args_roundtrip::<Cli>(500) {
        panic!("CLI argument roundtrip check failed:\n{e}")
    };
}
