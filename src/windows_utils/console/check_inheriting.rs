use tracing::debug;
use windows::Win32::System::Console::GetConsoleProcessList;

pub fn is_inheriting_console() -> bool {
    let mut pids = [0u32; 4]; // Buffer for at least two PIDs
    // https://learn.microsoft.com/en-us/windows/console/getconsoleprocesslist
    let count = unsafe { GetConsoleProcessList(pids.as_mut_slice()) };

    // count == 0: GetConsoleProcessList failed or no console is attached.
    // count == 1: Only our process is attached (e.g., we called AllocConsole,
    //             or a console app started in its own new window and we are that app).
    // count > 1: More than one process, implies we inherited from a parent (e.g., shell).
    let inheriting = count > 1;

    debug!(?pids, count, inheriting, "GetConsoleProcessList",);

    // For very early diagnostics before tracing is set up:
    // eprintln!("[is_inheriting_console] GetConsoleProcessList count: {count}, inheriting: {inheriting}");
    inheriting
}

#[cfg(test)]
mod test {
    #[test]
    fn it_works() -> eyre::Result<()> {
        let inheriting = super::is_inheriting_console();
        assert!(
            inheriting,
            "This test should be run from a console instead of double-clicking the exe."
        );
        Ok(())
    }
}
