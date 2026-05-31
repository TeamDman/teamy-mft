use crate::windows_utils::log::LOG_BUFFER;
use eyre::Context;
use eyre::ContextCompat;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::io::Write;
use std::process::Child;
use tracing::debug;
use tracing::error;

pub trait IoHook {
    fn hook_stdout_logs(&mut self) -> eyre::Result<()>;
    fn hook_stderr_logs(&mut self) -> eyre::Result<()>;
    fn hook_stdio_logs(&mut self) -> eyre::Result<()> {
        self.hook_stdout_logs()?;
        self.hook_stderr_logs()?;
        Ok(())
    }
}
impl IoHook for Child {
    fn hook_stdout_logs(&mut self) -> eyre::Result<()> {
        hook_stdout_logs(self).wrap_err("Failed to hook stdout logs")
    }

    fn hook_stderr_logs(&mut self) -> eyre::Result<()> {
        hook_stderr_logs(self).wrap_err("Failed to hook stderr logs")
    }
}

pub fn hook_stdout_logs(child: &mut Child) -> eyre::Result<()> {
    debug!("Hooking stdout logs");
    let stdout = child.stdout.take().wrap_err("Failed to capture stdout")?;
    let reader = BufReader::new(stdout);
    hook_io(reader);
    Ok(())
}

pub fn hook_stderr_logs(child: &mut Child) -> eyre::Result<()> {
    debug!("Hooking stderr logs");
    let stderr = child.stderr.take().wrap_err("Failed to capture stderr")?;
    let reader = BufReader::new(stderr);
    hook_io(reader);
    Ok(())
}

fn hook_io<T: Read + Send + 'static>(reader: BufReader<T>) {
    std::thread::spawn(move || {
        if let Err(e) = hook_io_inner(reader) {
            error!("Error in IO logging thread: {:?}", e);
        }
    });
}
fn hook_io_inner<T: Read + Send + 'static>(reader: BufReader<T>) -> eyre::Result<()> {
    let mut log_buffer = LOG_BUFFER.clone();
    for line in reader.lines() {
        let line = line.wrap_err("Failed to read line from child stdout")?;
        println!("{}", line);
        log_buffer
            .write_fmt(format_args!("{line}\n"))
            .wrap_err("Failed to write to log buffer")?;
    }
    Ok(())
}
