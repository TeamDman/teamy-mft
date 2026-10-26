use mimalloc::MiMalloc;
use teamy_mft::windows_utils::elevation::ElevationCompleted;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> eyre::Result<()> {
    handle_elevation_completion(teamy_mft::main())
}

// cli[impl elevation.completion]
fn handle_elevation_completion(result: eyre::Result<()>) -> eyre::Result<()> {
    if let Err(error) = &result
        && let Some(completed) = error.downcast_ref::<ElevationCompleted>()
    {
        let exit_code = completed.exit_code;
        // The command stack has returned; release the report before any explicit exit.
        drop(result);
        if exit_code == 0 {
            return Ok(());
        }
        // Preserve the full Windows status without reporting the child's error again.
        std::process::exit(i32::from_ne_bytes(exit_code.to_ne_bytes()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::ElevationCompleted;
    use super::handle_elevation_completion;
    use std::os::windows::process::CommandExt;
    use std::process::ExitCode;
    use std::process::Termination;

    const EXIT_CASE: &str = "TEAMY_MFT_TEST_EXIT_CASE";

    #[test]
    fn exit_process_probe() {
        let Ok(case) = std::env::var(EXIT_CASE) else {
            return;
        };
        color_eyre::install().unwrap();
        let result = match case.as_str() {
            "success" => Ok(()),
            "error" => Err(eyre::eyre!("Failed launching elevated child")),
            code => Err(eyre::Report::new(ElevationCompleted {
                exit_code: code.parse().unwrap(),
            })
            .wrap_err("Command dispatch")
            .wrap_err("Application context")),
        };
        // Use Rust's Result reporting path, as returning from main would.
        let exit_code = handle_elevation_completion(result).report();
        std::process::exit(if exit_code == ExitCode::SUCCESS { 0 } else { 1 });
    }

    // cli[verify elevation.completion]
    #[test]
    fn entrypoint_reports_only_real_errors_and_preserves_exit_codes() {
        for (case, expected_code, expect_report) in [
            ("success", 0_u32, false),
            ("0", 0, false),
            ("7", 7, false),
            ("3221225477", 0xc000_0005, false),
            ("error", 1, true),
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::exit_process_probe", "--nocapture"])
                .env(EXIT_CASE, case)
                .env("NO_COLOR", "1")
                .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
                .output()
                .unwrap();
            let code = output
                .status
                .code()
                .expect("child should have an exit code");
            assert_eq!(
                u32::from_ne_bytes(code.to_ne_bytes()),
                expected_code,
                "{case}"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            if expect_report {
                assert!(
                    stderr.contains("Failed launching elevated child"),
                    "{stderr}"
                );
                assert!(stderr.contains("Error:"), "{stderr}");
                assert!(stderr.contains("Location:"), "{stderr}");
            } else {
                assert!(stderr.is_empty(), "{case}: {stderr}");
            }
        }
    }
}
