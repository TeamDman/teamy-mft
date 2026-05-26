$serviceName = "teamy-mft-daemon"

if (Get-Command teamy-mft -ErrorAction SilentlyContinue) {
    Write-Host "Stopping $serviceName before reinstalling..."
    & teamy-mft daemon stop
}
else {
    Write-Host "teamy-mft not yet on PATH; skipping daemon stop helper."
}

$env:TEAMY_MFT_BUILD_UNIX_MS = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds().ToString()
cargo install --path . --locked --force
