Write-Host -ForegroundColor Yellow "Running format check..."
cargo fmt --all -- --check

Write-Host -ForegroundColor Yellow "Running clippy lint check..."
cargo clippy --all-targets --all-features -- -D warnings

Write-Host -ForegroundColor Yellow "Running build..."
cargo build --all-features --verbose

Write-Host -ForegroundColor Yellow "Running tests..."
cargo test --all-features --verbose