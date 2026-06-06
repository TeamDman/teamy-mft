use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

fn main() {
    add_exe_resources();
    add_git_revision();
    add_build_unix_ms();
}

/// Embeds Windows resources (like application icon) into the executable.
fn add_exe_resources() {
    println!("cargo:rerun-if-changed=resources");

    embed_resource::compile("resources/app.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed resources");
}

/// In your code you can now access git revision using
/// ```rust
/// let git_rev = option_env!("GIT_REVISION").unwrap_or("unknown");
/// ```
fn add_git_revision() {
    // Try to get a short git revision; on failure, set to "unknown".
    #[allow(
        clippy::disallowed_methods,
        reason = "build.rs intentionally shells out to git for embed-time revision metadata"
    )]
    let rev = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| o.status.success().then_some(o.stdout))
        .and_then(|v| String::from_utf8(v).ok())
        .map_or_else(|| "unknown".to_string(), |s| s.trim().to_string());

    println!("cargo:rustc-env=GIT_REVISION={rev}");
}

fn add_build_unix_ms() {
    println!("cargo:rerun-if-env-changed=TEAMY_MFT_BUILD_UNIX_MS");
    let build_unix_ms = std::env::var("TEAMY_MFT_BUILD_UNIX_MS").unwrap_or_else(|_| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_millis()
            .to_string()
    });

    println!("cargo:rustc-env=BUILD_UNIX_MS={build_unix_ms}");
}
