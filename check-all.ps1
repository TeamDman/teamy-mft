Write-Host -ForegroundColor Yellow "Running format check..."
rustup run nightly -- cargo fmt --all
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host -ForegroundColor Yellow "Running clippy lint check..."
# cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-features -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host -ForegroundColor Yellow "Running build..."
cargo build --all-features --quiet
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host -ForegroundColor Yellow "Running tests..."
$metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$pkg = $metadata.packages | Where-Object { $_.name -eq "teamy-mft" }
$features = $pkg.features.PSObject.Properties.Name | Where-Object { $_ -notin @("default", "tracy") }
$featuresArg = if ($features) { @("--features", ($features -join ",")) } else { @() }
cargo test @featuresArg --quiet
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host -ForegroundColor Yellow "Running tracey validation..."
tracey query validate --deny warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }