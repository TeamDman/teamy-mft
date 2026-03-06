use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Args;
use std::ffi::OsString;

#[derive(Args, Default, Arbitrary, PartialEq, Debug)]
pub struct GlobalArgs {
    /// Enable debug logging
    #[clap(long, global = true)]
    pub debug: bool,

    #[clap(long, global = true, value_name = "FILTER")]
    pub log_filter: Option<String>,

    #[clap(long, global = true, value_name = "FILE")]
    pub log_file: Option<String>,

    /// Emit structured JSON logs alongside stderr output.
    /// Optionally specify a filename; if not provided, a timestamped filename will be generated.
    #[clap(
        long,
        global = true,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = false
    )]
    json: Option<String>,

    /// Console PID for console reuse (hidden)
    #[clap(long, hide = true, global = true)]
    pub console_pid: Option<u32>,
}

impl ToArgs for GlobalArgs {
    fn to_args(&self) -> Vec<OsString> {
        let mut args = Vec::new();
        if self.debug {
            args.push("--debug".into());
        }
        match &self.json {
            None => {}
            Some(s) if s.is_empty() => {
                args.push("--json".into());
            }
            Some(path) => {
                args.push("--json".into());
                args.push(path.into());
            }
        }
        if let Some(pid) = self.console_pid {
            args.push("--console-pid".into());
            args.push(pid.to_string().into());
        }
        args
    }
}
