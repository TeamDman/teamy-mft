//! CLI fuzzing tests using figue's arbitrary helper assertions.

use figue::from_slice;
use teamy_mft::cli::Cli;
use teamy_mft::cli::command::Command;

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
// tool[verify cli.help.describes-machine-install]
fn help_mentions_machine_install_command() {
    let result = from_slice::<Cli>(&["--help"]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.is_help());

    let help = err.help_text().expect("help text should be present");
    assert!(
        help.contains("install"),
        "help should mention the install command, got:\n{help}"
    );
}

#[test]
fn sync_omits_daemon_preference_by_default() {
    let cli = from_slice::<Cli>(&["sync"]).unwrap();
    let Command::Sync(args) = cli.command else {
        panic!("expected sync command");
    };

    assert!(!args.daemon);
    assert!(!args.no_daemon);
}

#[test]
fn sync_accepts_daemon_preference() {
    let cli = from_slice::<Cli>(&["sync", "--daemon"]).unwrap();
    let Command::Sync(args) = cli.command else {
        panic!("expected sync command");
    };

    assert!(args.daemon);
    assert!(!args.no_daemon);
}

#[test]
fn sync_keeps_no_daemon_preference() {
    let cli = from_slice::<Cli>(&["sync", "--no-daemon"]).unwrap();
    let Command::Sync(args) = cli.command else {
        panic!("expected sync command");
    };

    assert!(!args.daemon);
    assert!(args.no_daemon);
}
