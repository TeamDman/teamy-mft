[CmdletBinding(PositionalBinding = $false)]
param(
	[switch]$Release,
	[switch]$NoOpenProfiler,
	[switch]$Elevated,
	[Parameter(Position = 0, ValueFromRemainingArguments = $true)]
	[string[]]$QueryArgs
)

function Test-IsAdministrator {
	$currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
	$principal = [Security.Principal.WindowsPrincipal]::new($currentIdentity)
	return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Quote-ProcessArgument {
	param(
		[Parameter(Mandatory = $true)]
		[string]$Argument
	)

	if ($Argument -notmatch '[\s"]') {
		return $Argument
	}

	return '"' + ($Argument -replace '"', '\"') + '"'
}

if ($Elevated -and -not (Test-IsAdministrator)) {
	$powershellCommand = Get-Command pwsh.exe -ErrorAction SilentlyContinue
	if (-not $powershellCommand) {
		$powershellCommand = Get-Command powershell.exe -ErrorAction Stop
	}

	$arguments = @(
		'-NoProfile',
		'-ExecutionPolicy',
		'Bypass',
		'-File',
		$PSCommandPath
	)
	if ($Release) {
		$arguments += '-Release'
	}
	if ($NoOpenProfiler) {
		$arguments += '-NoOpenProfiler'
	}
	$arguments += '-Elevated'
	$arguments += $QueryArgs

	Write-Host "Relaunching profiler wrapper as administrator..."
	$process = Start-Process `
		-FilePath $powershellCommand.Source `
		-ArgumentList (($arguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' ') `
		-Verb RunAs `
		-Wait `
		-PassThru
	exit $process.ExitCode
}

$serviceName = "teamy-mft-daemon"

if (Get-Command teamy-mft -ErrorAction SilentlyContinue) {
    Write-Host "Stopping $serviceName before build..."
    & teamy-mft daemon stop
}
else {
    Write-Host "teamy-mft not yet on PATH; skipping daemon stop helper."
}

$profilerFeatures = 'tracy'

function Format-Elapsed {
	param(
		[Parameter(Mandatory = $true)]
		[TimeSpan]$Elapsed
	)

	if ($Elapsed.TotalHours -ge 1) {
		return $Elapsed.ToString("hh\:mm\:ss\.fff")
	}

	return $Elapsed.ToString("mm\:ss\.fff")
}

function Get-TracyCaptureProcesses {
	param(
		[Parameter(Mandatory = $true)]
		[string]$CapturePath
	)

	$slugPattern = [Regex]::Escape($CapturePath)
	Get-CimInstance Win32_Process -Filter "Name = 'tracy-capture.exe'" -ErrorAction SilentlyContinue |
		Where-Object { $_.CommandLine -and $_.CommandLine -match $slugPattern } |
		ForEach-Object {
			try {
				Get-Process -Id $_.ProcessId -ErrorAction Stop
			} catch {
				$null
			}
		} |
		Where-Object { $_ -ne $null }
}

function Wait-ForTracyCaptureReady {
	param(
		[Parameter(Mandatory = $true)]
		[string]$CapturePath,
		[Parameter(Mandatory = $true)]
		[TimeSpan]$Timeout
	)

	$deadline = (Get-Date).Add($Timeout)
	do {
		$processes = @(Get-TracyCaptureProcesses -CapturePath $CapturePath)
		if ($processes.Count -gt 0) {
			return $processes
		}

		Start-Sleep -Milliseconds 250
	} while ((Get-Date) -lt $deadline)

	throw "Timed out waiting $(Format-Elapsed $Timeout) for tracy-capture to start for $CapturePath"
}

function Wait-ForTracyCaptureExit {
	param(
		[Parameter(Mandatory = $true)]
		[string]$CapturePath,
		[Parameter(Mandatory = $true)]
		[TimeSpan]$Timeout,
		[switch]$AlreadyObserved
	)

	$waitStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	$deadline = (Get-Date).Add($Timeout)
	$observedProcess = [bool]$AlreadyObserved
	do {
		$processes = @(Get-TracyCaptureProcesses -CapturePath $CapturePath)
		if ($processes.Count -gt 0) {
			$observedProcess = $true
		}
		if ($processes.Count -eq 0) {
			$waitStopwatch.Stop()
			if ($observedProcess) {
				return $waitStopwatch.Elapsed
			}
			return $null
		}

		Start-Sleep -Milliseconds 250
	} while ((Get-Date) -lt $deadline)

	$waitStopwatch.Stop()
	return $null
}

function Start-TracyCaptureProcess {
	param(
		[Parameter(Mandatory = $true)]
		[string]$CapturePath
	)

	$wt = Get-Command wt.exe -ErrorAction SilentlyContinue
	if ($wt) {
		try {
			Start-Process -FilePath "wt.exe" -ArgumentList @("-w", "new", "tracy-capture.exe", "-o", $CapturePath) -ErrorAction Stop
			return
		} catch {
			Write-Warning "Failed to launch tracy-capture in Windows Terminal: $($_.Exception.Message)"
			Write-Warning "Falling back to launching tracy-capture directly."
		}
	} else {
		Write-Warning "wt.exe not found in PATH; launching tracy-capture directly"
	}

	Start-Process -FilePath "tracy-capture.exe" -ArgumentList @("-o", $CapturePath) -PassThru -ErrorAction Stop
}

function Stop-TracyCaptureGracefully {
	param(
		[Parameter(Mandatory = $true)]
		[string]$CapturePath
	)

	$processes = @(Get-TracyCaptureProcesses -CapturePath $CapturePath)
	if ($processes.Count -eq 0) {
		return [TimeSpan]::Zero
	}

	$shutdownStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	foreach ($process in $processes) {
		if ($process.HasExited) {
			continue
		}

		$requestedClose = $false
		try {
			$requestedClose = $process.CloseMainWindow()
		} catch {
			$requestedClose = $false
		}

		if (-not $requestedClose) {
			try {
				Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
			} catch {
				# Ignore shutdown failures and let the wait/kill fallback below handle them.
			}
		}
	}

	$waitDeadline = (Get-Date).AddSeconds(30)
	do {
		Start-Sleep -Milliseconds 250
		$processes = @($processes | Where-Object {
			try {
				$_.Refresh()
				-not $_.HasExited
			} catch {
				$false
			}
		})
	} while ($processes.Count -gt 0 -and (Get-Date) -lt $waitDeadline)

	foreach ($process in $processes) {
		try {
			if (-not $process.HasExited) {
				$process.Kill()
			}
		} catch {
			# Ignore final cleanup failures.
		}
	}

	$shutdownStopwatch.Stop()
	return $shutdownStopwatch.Elapsed
}

$overallStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
$buildElapsed = $null
$captureLaunchElapsed = $null
$commandElapsed = $null
$cleanupElapsed = $null
$profilerElapsed = $null
$captureShutdownElapsed = [TimeSpan]::Zero
$captureFlushDelay = [TimeSpan]::FromSeconds(1)
$captureStartupTimeout = [TimeSpan]::FromSeconds(10)
$captureExitTimeout = [TimeSpan]::FromMinutes(5)
$captureAttachDelay = [TimeSpan]::FromMilliseconds(250)
$commandExitCode = 0
$commandFailureMessage = $null
$captureReadyForPostProcessing = $false
$capturePostProcessingSkipReason = $null

if (-not (Get-Command tracy-capture.exe -ErrorAction SilentlyContinue)) {
	throw "tracy-capture.exe not found in PATH"
}

$profilerCommand = Get-Command tracy-profiler.exe -ErrorAction SilentlyContinue
$csvExportCommand = Get-Command tracy-csvexport.exe -ErrorAction SilentlyContinue

if (-not $NoOpenProfiler -and -not $profilerCommand) {
	Write-Warning "tracy-profiler.exe not found in PATH; capture will still be produced"
}

if (-not $csvExportCommand) {
	Write-Warning "tracy-csvexport.exe not found in PATH; CSV export will be skipped"
}

if (-not $QueryArgs -or $QueryArgs.Count -eq 0) {
	$QueryArgs = @("status")
}

$profileOutputDirectory = if ($Release) { 'release' } else { 'debug' }
$profileLabel = if ($Release) { 'release' } else { 'debug' }
$buildArgs = @('build', '--bin', 'teamy-mft', '--features', $profilerFeatures)
if ($Release) {
	$buildArgs += '--release'
}
$teamyMftPath = Join-Path $PSScriptRoot "target\$profileOutputDirectory\teamy-mft.exe"
$appArgs = @($QueryArgs)

$buildStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
Write-Host "Building $profileLabel with features ${profilerFeatures}: cargo $($buildArgs -join ' ')"
& cargo @buildArgs
$buildStopwatch.Stop()
$buildElapsed = $buildStopwatch.Elapsed
Write-Host "Build time: $(Format-Elapsed $buildElapsed)"
if ($LASTEXITCODE -ne 0) {
	throw "cargo build failed with exit code $LASTEXITCODE"
}

if (-not (Test-Path $teamyMftPath)) {
	throw "built teamy-mft executable not found at $teamyMftPath"
}

$captureDir = Join-Path $PSScriptRoot "tracy"
if (-not (Test-Path $captureDir)) {
	$null = New-Item -ItemType Directory -Path $captureDir
}

$slug = "$((Get-Date).ToString("yyyy-MM-dd_HH-mm-ss")).tracy"
$capturePath = Join-Path $captureDir $slug

Write-Host "Capture: $capturePath"
Write-Host "Logging teamy-mft runtime performance information to $capturePath"
$capture = $null

try {
	Write-Host "Starting built $profileLabel teamy-mft: $teamyMftPath $($appArgs -join ' ')"
	$commandStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	$teamyProcess = Start-Process -FilePath $teamyMftPath -ArgumentList $appArgs -NoNewWindow -PassThru
	Write-Host "Waiting $(Format-Elapsed $captureAttachDelay) for teamy-mft Tracy endpoint before launching capture"
	Start-Sleep -Milliseconds ([int]$captureAttachDelay.TotalMilliseconds)

	$captureLaunchStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	$capture = Start-TracyCaptureProcess -CapturePath $capturePath
	$captureLaunchStopwatch.Stop()
	$captureLaunchElapsed = $captureLaunchStopwatch.Elapsed
	Write-Host "Capture launch time: $(Format-Elapsed $captureLaunchElapsed)"
	Write-Host "Waiting for tracy-capture process to appear (timeout $(Format-Elapsed $captureStartupTimeout))"
	$captureProcesses = @(Wait-ForTracyCaptureReady -CapturePath $capturePath -Timeout $captureStartupTimeout)
	Write-Host "tracy-capture ready (pid: $($captureProcesses.Id -join ', '))"

	$teamyProcess.WaitForExit()
	$commandStopwatch.Stop()
	$commandElapsed = $commandStopwatch.Elapsed
	$commandExitCode = $teamyProcess.ExitCode
	Write-Host "Traced command time: $(Format-Elapsed $commandElapsed)"
	if ($commandExitCode -ne 0) {
		$commandFailureMessage = "teamy-mft.exe failed with exit code $commandExitCode"
		Write-Warning $commandFailureMessage
	}
}
finally {
	$cleanupStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	Write-Host "Waiting $(Format-Elapsed $captureFlushDelay) before watching tracy-capture shutdown"
	Start-Sleep -Milliseconds ([int]$captureFlushDelay.TotalMilliseconds)
	$naturalCaptureShutdownElapsed = Wait-ForTracyCaptureExit -CapturePath $capturePath -Timeout $captureExitTimeout -AlreadyObserved
	if ($null -ne $naturalCaptureShutdownElapsed) {
		$captureShutdownElapsed = $naturalCaptureShutdownElapsed
		$captureReadyForPostProcessing = $true
		Write-Host "tracy-capture exited after the client disconnected"
	} else {
		$capturePostProcessingSkipReason = "tracy-capture did not finish saving within $(Format-Elapsed $captureExitTimeout)"
		Write-Warning "Timed out waiting $(Format-Elapsed $captureExitTimeout) for tracy-capture to finish saving; forcing shutdown"
		$captureShutdownElapsed = Stop-TracyCaptureGracefully -CapturePath $capturePath
	}
	$cleanupStopwatch.Stop()
	$cleanupElapsed = $cleanupStopwatch.Elapsed
	Write-Host "Capture cleanup time: $(Format-Elapsed $cleanupElapsed)"
	Write-Host "Capture shutdown wait: $(Format-Elapsed $captureShutdownElapsed)"
}

if ($captureReadyForPostProcessing -and -not (Test-Path $capturePath)) {
	$captureReadyForPostProcessing = $false
	$capturePostProcessingSkipReason = "tracy-capture exited but no capture file was written to $capturePath"
}

if ($NoOpenProfiler) {
	if ($captureReadyForPostProcessing) {
		Write-Host "Skipping tracy-profiler launch (-NoOpenProfiler). Capture saved to $capturePath"
	} else {
		Write-Warning "Skipping tracy-profiler launch (-NoOpenProfiler). $capturePostProcessingSkipReason"
	}
} elseif ($profilerCommand -and $captureReadyForPostProcessing) {
	Write-Host "Displaying results from $capturePath"
	$profilerStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	tracy-profiler.exe "$capturePath"
	$profilerStopwatch.Stop()
	$profilerElapsed = $profilerStopwatch.Elapsed
	Write-Host "Profiler time: $(Format-Elapsed $profilerElapsed)"
} else {
	if ($captureReadyForPostProcessing) {
		Write-Host "Capture saved to $capturePath"
	} else {
		Write-Warning "Skipping tracy-profiler launch because $capturePostProcessingSkipReason"
	}
}

$overallStopwatch.Stop()

if ($csvExportCommand -and $captureReadyForPostProcessing) {
	Write-Host "CSV from tracy-csvexport.exe $capturePath"
	tracy-csvexport.exe $capturePath
} elseif ($csvExportCommand) {
	Write-Warning "Skipping tracy-csvexport because $capturePostProcessingSkipReason"
}

Write-Host "Timing summary:"
if ($buildElapsed) {
	Write-Host "  build:          $(Format-Elapsed $buildElapsed)"
}
if ($captureLaunchElapsed) {
	Write-Host "  capture launch: $(Format-Elapsed $captureLaunchElapsed)"
}
if ($commandElapsed) {
	Write-Host "  traced command: $(Format-Elapsed $commandElapsed)"
}
Write-Host "  cleanup:        $(Format-Elapsed $cleanupElapsed)"
Write-Host "  capture stop:   $(Format-Elapsed $captureShutdownElapsed)"
if ($profilerElapsed) {
	Write-Host "  profiler:       $(Format-Elapsed $profilerElapsed)"
}
Write-Host "  total wrapper:  $(Format-Elapsed $overallStopwatch.Elapsed)"

if ($commandFailureMessage) {
	throw $commandFailureMessage
}
