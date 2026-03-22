param(
	[Parameter(ValueFromRemainingArguments = $true)]
	[string[]]$QueryArgs
)

$captureDir = Join-Path $PSScriptRoot "tracy"
if (-not (Test-Path $captureDir)) {
	$null = New-Item -ItemType Directory -Path $captureDir
}

$slug = "$((Get-Date).ToString("yyyy-MM-dd_HH-mm-ss")).tracy"
$capturePath = Join-Path $captureDir $slug

if (-not (Get-Command tracy-capture.exe -ErrorAction SilentlyContinue)) {
	throw "tracy-capture.exe not found in PATH"
}

if (-not (Get-Command tracy-profiler.exe -ErrorAction SilentlyContinue)) {
	Write-Warning "tracy-profiler.exe not found in PATH; capture will still be produced at $capturePath"
}

if (-not $QueryArgs -or $QueryArgs.Count -eq 0) {
	$QueryArgs = @("query", "'flower .jar$")
}

$isSyncCommand = $QueryArgs.Count -gt 0 -and $QueryArgs[0] -eq "sync"

Write-Host "Capture: $capturePath"
Write-Host "Logging performance information to $capturePath"
$capture = $null
$wt = Get-Command wt.exe -ErrorAction SilentlyContinue

if ($wt) {
	Start-Process -FilePath "wt.exe" -ArgumentList @("-w", "new", "tracy-capture.exe", "-o", $capturePath)
} else {
	Write-Warning "wt.exe not found in PATH; launching tracy-capture in the current session"
	$capture = Start-Process -FilePath "tracy-capture.exe" -ArgumentList @("-o", $capturePath) -PassThru
}

try {
	if ($isSyncCommand) {
		$binaryPath = Join-Path $PSScriptRoot "target\release\teamy-mft.exe"
		Write-Host "Building Tracy-enabled release binary for elevated sync tracing"
		cargo build --release --features tracy
		if ($LASTEXITCODE -ne 0) {
			throw "cargo build failed with exit code $LASTEXITCODE"
		}

		if (-not (Test-Path $binaryPath)) {
			throw "Expected binary not found at $binaryPath"
		}

		$arguments = @($QueryArgs + "--log-filter debug")
		Write-Host "Running elevated: $binaryPath $($arguments -join ' ')"
		Start-Process -FilePath $binaryPath -ArgumentList $arguments -Verb RunAs -Wait
	} else {
		Write-Host "Running: cargo run --release --features tracy -- $($QueryArgs -join ' ')"
		cargo run --release --features tracy -- @QueryArgs --log-filter debug
		if ($LASTEXITCODE -ne 0) {
			throw "cargo run failed with exit code $LASTEXITCODE"
		}
	}
}
finally {
	$slugPattern = [Regex]::Escape($capturePath)
	Get-CimInstance Win32_Process -Filter "Name = 'tracy-capture.exe'" -ErrorAction SilentlyContinue |
		Where-Object { $_.CommandLine -and $_.CommandLine -match $slugPattern } |
		ForEach-Object { Stop-Process -Id $_.ProcessId -ErrorAction SilentlyContinue }

	if ($capture -and -not $capture.HasExited) {
		Start-Sleep -Milliseconds 10000
		$null = $capture.CloseMainWindow()
		Start-Sleep -Milliseconds 500
		if (-not $capture.HasExited) {
			$capture.Kill()
		}
	}
}

if (Get-Command tracy-profiler.exe -ErrorAction SilentlyContinue) {
	Write-Host "Displaying results from $capturePath"
	tracy-profiler.exe "$capturePath"
} else {
	Write-Host "Capture saved to $capturePath"
}