use crate::cli::global_args::GlobalArgs;
use crate::cli::to_args::ToArgs;
use crate::engine::run::run_engine;
use arbitrary::Arbitrary;
use clap::Args;

#[derive(Args, Arbitrary, PartialEq, Debug)]
pub struct UiArgs {}

impl UiArgs {
    pub fn invoke(self, global_args: GlobalArgs) -> eyre::Result<()> {
        run_engine(global_args)?;
        Ok(())
    }
}

impl ToArgs for UiArgs {
    fn to_args(&self) -> Vec<std::ffi::OsString> {
        vec![]
    }
}
