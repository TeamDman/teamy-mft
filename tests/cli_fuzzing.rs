//! CLI fuzzing tests using figue's arbitrary helper assertions.

use teamy_mft::cli::Cli;

#[test]
fn fuzz_cli_args_consistency() {
    if let Err(e) =
        figue::assert_to_args_consistency::<Cli>(figue::TestToArgsConsistencyConfig::default())
    {
        panic!("CLI argument consistency check failed:\n{e}")
    }
}

#[test]
fn fuzz_cli_args_roundtrip() {
    // Retain the seed that exposed missing reflected equality for the opaque limit.
    for root_seed in [Some(12_793_442_261_061_576_120), None] {
        let config = figue::TestToArgsRoundTrip {
            root_seed,
            ..Default::default()
        };
        if let Err(e) = figue::assert_to_args_roundtrip::<Cli>(config) {
            panic!("CLI argument roundtrip check failed:\n{e}")
        }
    }
}
