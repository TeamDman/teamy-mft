use teamy_mft::cancellation::CancellationToken;
use teamy_mft::cli::global_args::GlobalArgs;
use teamy_mft::logging_init::LogSource;

#[test]
fn logging_initialization_enables_error_span_capture() {
    color_eyre::install().unwrap();
    teamy_mft::logging_init::init_logging(
        &GlobalArgs::default(),
        LogSource::Client,
        CancellationToken::new(),
    )
    .unwrap();
    let _span = tracing::info_span!("captured_sync_failure", drive = "G").entered();
    let report =
        eyre::eyre!("Run length field exceeds buffer").wrap_err("Failed decoding data run entry");
    let rendered = format!("{report:?}");
    assert!(rendered.contains("captured_sync_failure"), "{rendered}");
    assert!(!rendered.contains("Unsupported"), "{rendered}");
    assert_eq!(
        format!("{report:#}"),
        "Failed decoding data run entry: Run length field exceeds buffer"
    );
}
