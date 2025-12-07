use crate::cli::json_log_behaviour::JsonLogBehaviour;
use crate::cli::to_args::ToArgs;
use arbitrary::Arbitrary;
use clap::Args;
use std::ffi::OsString;

#[derive(Args, Default, Arbitrary, PartialEq, Debug)]
pub struct GlobalArgs {
    /// Enable debug logging
    #[clap(long, global = true)]
    pub debug: bool,

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

impl GlobalArgs {
    #[must_use]
    pub fn log_level(&self) -> tracing::Level {
        if self.debug {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        }
    }

    /// Determine how JSON structured logs should be handled based on the --json flag.
    #[must_use]
    pub fn json_log_behaviour(&self) -> JsonLogBehaviour {
        match &self.json {
            None => JsonLogBehaviour::None,
            Some(s) if s.is_empty() => JsonLogBehaviour::SomeAutomaticPath,
            Some(path) => JsonLogBehaviour::Some(path.into()),
        }
    }
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
