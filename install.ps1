$serviceName = "teamy-mft-daemon"

if (Get-Command teamy-mft -ErrorAction SilentlyContinue) {
    Write-Host "Stopping $serviceName before reinstalling..."
    & teamy-mft daemon stop
}
else {
    Write-Host "teamy-mft not yet on PATH; skipping daemon stop helper."
}

cargo install --path . --locked --force
