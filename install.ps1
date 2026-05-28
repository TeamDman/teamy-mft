if (Get-Command teamy-mft -ErrorAction SilentlyContinue) {
    Write-Host "Stopping teamy-mft-daemon service before reinstalling..."
    & teamy-mft daemon stop
}
else {
    Write-Host "teamy-mft not yet on PATH; skipping daemon stop helper."
}

$runningClients = @(Get-Process teamy-mft -ErrorAction SilentlyContinue)
if ($runningClients.Count -gt 0) {
    Start-Sleep -Seconds 1
}
$runningClients = @(Get-Process teamy-mft -ErrorAction SilentlyContinue)
if ($runningClients.Count -gt 0) {
    Write-Warning "teamy-mft.exe is still running after daemon stop. Close any 'teamy-mft daemon logs -f' or query terminals before reinstalling."
    $runningClients | Select-Object Id, ProcessName, Path | Format-Table -AutoSize
    exit 1
}

$env:TEAMY_MFT_BUILD_UNIX_MS = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds().ToString()
cargo install --path . --locked --force --offline
