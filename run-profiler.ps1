[CmdletBinding(PositionalBinding = $false)]
param(
	[switch]$Release,
	[switch]$NoOpenProfiler,
	[switch]$RowLevelInsight,
	[string]$Example,
	[switch]$Elevated,
	[switch]$ElevateTracy,
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

if (-not ("TeamyMftProfilerLoggedProcess" -as [type])) {
	Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading.Tasks;

public sealed class TeamyMftProfilerLoggedProcess : IDisposable
{
    private readonly object gate = new object();
    private readonly Process process;
    private readonly StreamWriter logWriter;
    private readonly Task stdoutTask;
    private readonly Task stderrTask;
    private bool closed;

    private TeamyMftProfilerLoggedProcess(Process process, StreamWriter logWriter)
    {
        this.process = process;
        this.logWriter = logWriter;
        stdoutTask = Task.Factory.StartNew(delegate { Pump(process.StandardOutput); }, TaskCreationOptions.LongRunning);
        stderrTask = Task.Factory.StartNew(delegate { Pump(process.StandardError); }, TaskCreationOptions.LongRunning);
    }

    public Process Process
    {
        get { return process; }
    }

    public static TeamyMftProfilerLoggedProcess Start(string filePath, string arguments, string logPath)
    {
        string logDirectory = Path.GetDirectoryName(logPath);
        if (!String.IsNullOrEmpty(logDirectory))
        {
            Directory.CreateDirectory(logDirectory);
        }

        StreamWriter logWriter = new StreamWriter(logPath, false, new UTF8Encoding(false));
        logWriter.AutoFlush = true;
        logWriter.WriteLine("# started: " + DateTimeOffset.Now.ToString("o"));
        if (String.IsNullOrWhiteSpace(arguments))
        {
            logWriter.WriteLine("# command: " + filePath);
        }
        else
        {
            logWriter.WriteLine("# command: " + filePath + " " + arguments);
        }
        logWriter.WriteLine();

        ProcessStartInfo startInfo = new ProcessStartInfo();
        startInfo.FileName = filePath;
        startInfo.Arguments = arguments;
        startInfo.UseShellExecute = false;
        startInfo.RedirectStandardOutput = true;
        startInfo.RedirectStandardError = true;
        startInfo.CreateNoWindow = false;

        Process process = new Process();
        process.StartInfo = startInfo;
        try
        {
            process.Start();
        }
        catch
        {
            logWriter.Dispose();
            process.Dispose();
            throw;
        }

        return new TeamyMftProfilerLoggedProcess(process, logWriter);
    }

    private void Pump(StreamReader reader)
    {
        try
        {
            string line;
            while ((line = reader.ReadLine()) != null)
            {
                WriteLine(line);
            }
        }
        catch (ObjectDisposedException)
        {
        }
        catch (IOException)
        {
        }
    }

    private void WriteLine(string line)
    {
        lock (gate)
        {
            logWriter.WriteLine(line);
            Console.WriteLine(line);
        }
    }

    public int WaitForExit()
    {
        process.WaitForExit();
        Task.WaitAll(stdoutTask, stderrTask);
        return process.ExitCode;
    }

    public void Close()
    {
        if (closed)
        {
            return;
        }

        try
        {
            if (!process.HasExited)
            {
                process.Kill();
            }
        }
        catch
        {
        }

        try
        {
            WaitForExit();
        }
        catch
        {
        }

        lock (gate)
        {
            logWriter.Flush();
            logWriter.Dispose();
        }

        process.Dispose();
        closed = true;
    }

    public void Dispose()
    {
        Close();
    }
}
'@
}

function Start-LoggedProcess {
	param(
		[Parameter(Mandatory = $true)]
		[string]$FilePath,
		[string[]]$ArgumentList,
		[Parameter(Mandatory = $true)]
		[string]$LogPath
	)

	if ($null -eq $ArgumentList) {
		$ArgumentList = @()
	}
	$ArgumentList = @($ArgumentList | Where-Object { $null -ne $_ -and $_ -ne '' })

	$quotedArguments = (($ArgumentList | ForEach-Object { Quote-ProcessArgument $_ }) -join ' ')
	return [TeamyMftProfilerLoggedProcess]::Start($FilePath, $quotedArguments, $LogPath)
}

function Wait-LoggedProcessExit {
	param(
		[Parameter(Mandatory = $true)]
		$LoggedProcess
	)

	return $LoggedProcess.WaitForExit()
}

function Close-LoggedProcess {
	param(
		$LoggedProcess
	)

	if ($null -eq $LoggedProcess) {
		return
	}

	$LoggedProcess.Close()
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
	if ($RowLevelInsight) {
		$arguments += '-RowLevelInsight'
	}
	if ($Example) {
		$arguments += '-Example'
		$arguments += $Example
	}
	if ($ElevateTracy) {
		$arguments += '-ElevateTracy'
	}
	$arguments += '-Elevated'
	$arguments += $QueryArgs

	Write-Host "Relaunching profiler wrapper as administrator..."
	$wt = Get-Command wt.exe -ErrorAction SilentlyContinue
	if ($wt) {
		$wtArguments = @(
			'-w',
			'new',
			'new-tab',
			'--title',
			'teamy-mft profiler (admin)',
			'-d',
			$PSScriptRoot,
			$powershellCommand.Source
		) + $arguments
		try {
			$process = Start-Process `
				-FilePath $wt.Source `
				-ArgumentList (($wtArguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' ') `
				-Verb RunAs `
				-Wait `
				-PassThru
			exit $process.ExitCode
		} catch {
			Write-Warning "Failed to relaunch profiler wrapper in Windows Terminal: $($_.Exception.Message)"
			Write-Warning "Falling back to launching elevated PowerShell directly."
		}
	} else {
		Write-Warning "wt.exe not found in PATH; launching elevated PowerShell directly."
	}

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

$profilerFeatures = @('extended_observability')
if ($RowLevelInsight) {
	$profilerFeatures += 'extended_observability_per_record'
}
$profilerFeatureArgument = $profilerFeatures -join ' '
$profilerFeatureLabel = $profilerFeatures -join ', '

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
		[string]$CapturePath,
		[switch]$ElevateTracy
	)

	$tracyCaptureCommand = Get-Command tracy-capture.exe -ErrorAction Stop
	$tracyCapturePath = $tracyCaptureCommand.Source
	$tracyCaptureArguments = @("-o", $CapturePath)
	$tracyCaptureArgumentString = (($tracyCaptureArguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' ')
	$isAdministrator = Test-IsAdministrator
	$wt = Get-Command wt.exe -ErrorAction SilentlyContinue

	if ($isAdministrator -and -not $ElevateTracy) {
		Write-Host "Launching tracy-capture without elevation. Pass -ElevateTracy to inherit the administrator token."
		$shell = New-Object -ComObject Shell.Application
		if ($wt) {
			$wtArguments = @("-w", "new", $tracyCapturePath) + $tracyCaptureArguments
			$wtArgumentString = (($wtArguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' ')
			try {
				$shell.ShellExecute($wt.Source, $wtArgumentString, $PSScriptRoot, "open", 1)
				return
			} catch {
				Write-Warning "Failed to launch unelevated tracy-capture in Windows Terminal: $($_.Exception.Message)"
				Write-Warning "Falling back to launching unelevated tracy-capture directly."
			}
		} else {
			Write-Warning "wt.exe not found in PATH; launching unelevated tracy-capture directly"
		}

		try {
			$shell.ShellExecute($tracyCapturePath, $tracyCaptureArgumentString, $PSScriptRoot, "open", 1)
			return
		} catch {
			throw "Failed to launch tracy-capture without elevation: $($_.Exception.Message)"
		}
	}

	if ($ElevateTracy -and -not $isAdministrator) {
		if ($wt) {
			$wtArguments = @("-w", "new", $tracyCapturePath) + $tracyCaptureArguments
			try {
				Start-Process `
					-FilePath $wt.Source `
					-ArgumentList (($wtArguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' ') `
					-Verb RunAs `
					-ErrorAction Stop
				return
			} catch {
				Write-Warning "Failed to launch elevated tracy-capture in Windows Terminal: $($_.Exception.Message)"
				Write-Warning "Falling back to launching elevated tracy-capture directly."
			}
		} else {
			Write-Warning "wt.exe not found in PATH; launching elevated tracy-capture directly"
		}

		Start-Process `
			-FilePath $tracyCapturePath `
			-ArgumentList $tracyCaptureArgumentString `
			-Verb RunAs `
			-PassThru `
			-ErrorAction Stop
		return
	}

	if ($wt) {
		$wtArguments = @("-w", "new", $tracyCapturePath) + $tracyCaptureArguments
		try {
			Start-Process -FilePath $wt.Source -ArgumentList (($wtArguments | ForEach-Object { Quote-ProcessArgument $_ }) -join ' ') -ErrorAction Stop
			return
		} catch {
			Write-Warning "Failed to launch tracy-capture in Windows Terminal: $($_.Exception.Message)"
			Write-Warning "Falling back to launching tracy-capture directly."
		}
	} else {
		Write-Warning "wt.exe not found in PATH; launching tracy-capture directly"
	}

	Start-Process -FilePath $tracyCapturePath -ArgumentList $tracyCaptureArgumentString -PassThru -ErrorAction Stop
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


if ($QueryArgs.Count -gt 0 -and $QueryArgs[0] -eq '--') {
	$QueryArgs = @($QueryArgs | Select-Object -Skip 1)
}

if (-not $Example -and (-not $QueryArgs -or $QueryArgs.Count -eq 0)) {
	$QueryArgs = @("status")
}

$profileOutputDirectory = if ($Release) { 'release' } else { 'debug' }
$profileLabel = if ($Release) { 'release' } else { 'debug' }
$targetPath = $null
$targetLabel = $null
$failureTargetLabel = $null
$buildTargetArgs = @()
if ($Example) {
	$buildTargetArgs = @('--example', $Example)
	$targetPath = Join-Path $PSScriptRoot "target\$profileOutputDirectory\examples\$Example.exe"
	$targetLabel = "example $Example"
	$failureTargetLabel = "$Example.exe"
} else {
	$buildTargetArgs = @('--bin', 'teamy-mft')
	$targetPath = Join-Path $PSScriptRoot "target\$profileOutputDirectory\teamy-mft.exe"
	$targetLabel = 'teamy-mft'
	$failureTargetLabel = 'teamy-mft.exe'
}

$buildArgs = @('build') + $buildTargetArgs + @('--features', $profilerFeatureArgument)
if ($Release) {
	$buildArgs += '--release'
}
$appArgs = @($QueryArgs)

$buildStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
Write-Host "Building $profileLabel with features ${profilerFeatureLabel}: cargo $($buildArgs -join ' ')"
& cargo @buildArgs
$buildStopwatch.Stop()
$buildElapsed = $buildStopwatch.Elapsed
Write-Host "Build time: $(Format-Elapsed $buildElapsed)"
if ($LASTEXITCODE -ne 0) {
	throw "cargo build failed with exit code $LASTEXITCODE"
}

if (-not (Test-Path $targetPath)) {
	throw "built $targetLabel executable not found at $targetPath"
}

$captureDir = Join-Path $PSScriptRoot "tracy"
if (-not (Test-Path $captureDir)) {
	$null = New-Item -ItemType Directory -Path $captureDir
}

$slug = "$((Get-Date).ToString("yyyy-MM-dd_HH-mm-ss")).tracy"
$capturePath = Join-Path $captureDir $slug
$logPath = [System.IO.Path]::ChangeExtension($capturePath, ".log")

Write-Host "Capture: $capturePath"
Write-Host "Logging teamy-mft runtime performance information to $capturePath"
Write-Host "Log: $logPath"
$capture = $null
$loggedTeamyProcess = $null

try {
	Write-Host "Starting built $profileLabel ${targetLabel}: $targetPath $($appArgs -join ' ')"
	$commandStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	$loggedTeamyProcess = Start-LoggedProcess -FilePath $targetPath -ArgumentList $appArgs -LogPath $logPath
	$teamyProcess = $loggedTeamyProcess.Process
	Write-Host "Waiting $(Format-Elapsed $captureAttachDelay) for $targetLabel Tracy endpoint before launching capture"
	Start-Sleep -Milliseconds ([int]$captureAttachDelay.TotalMilliseconds)

	$captureLaunchStopwatch = [System.Diagnostics.Stopwatch]::StartNew()
	$capture = Start-TracyCaptureProcess -CapturePath $capturePath -ElevateTracy:$ElevateTracy
	$captureLaunchStopwatch.Stop()
	$captureLaunchElapsed = $captureLaunchStopwatch.Elapsed
	Write-Host "Capture launch time: $(Format-Elapsed $captureLaunchElapsed)"
	Write-Host "Waiting for tracy-capture process to appear (timeout $(Format-Elapsed $captureStartupTimeout))"
	$captureProcesses = @(Wait-ForTracyCaptureReady -CapturePath $capturePath -Timeout $captureStartupTimeout)
	Write-Host "tracy-capture ready (pid: $($captureProcesses.Id -join ', '))"

	$commandExitCode = Wait-LoggedProcessExit -LoggedProcess $loggedTeamyProcess
	$commandStopwatch.Stop()
	$commandElapsed = $commandStopwatch.Elapsed
	Close-LoggedProcess -LoggedProcess $loggedTeamyProcess
	$loggedTeamyProcess = $null
	Write-Host "Traced command time: $(Format-Elapsed $commandElapsed)"
	if ($commandExitCode -ne 0) {
		$commandFailureMessage = "$failureTargetLabel failed with exit code $commandExitCode"
		Write-Warning $commandFailureMessage
	}
}
finally {
	if ($loggedTeamyProcess) {
		Close-LoggedProcess -LoggedProcess $loggedTeamyProcess
		$loggedTeamyProcess = $null
	}
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
	Write-Error $commandFailureMessage
	Pause
	throw $commandFailureMessage
} else {
	if ($Elevated) {
		Pause
	}
}
