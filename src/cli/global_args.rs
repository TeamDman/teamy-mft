use arbitrary::Arbitrary;
use facet::Facet;
use figue::{self as args};

#[derive(Facet, Default, Arbitrary, PartialEq, Debug)]
#[facet(rename_all = "kebab-case")]
pub struct GlobalArgs {
    /// Enable debug logging
    #[facet(args::named, default)]
    pub debug: bool,

    #[facet(args::named)]
    pub log_filter: Option<String>,

    #[facet(args::named)]
    pub log_file: Option<String>,

    /// Emit structured JSON logs alongside stderr output.
    /// Optionally specify a filename; if not provided, a timestamped filename will be generated.
    #[facet(args::named)]
    pub json: Option<String>,

    /// Console PID for console reuse (hidden)
    #[facet(args::named)]
    pub console_pid: Option<u32>,
}
