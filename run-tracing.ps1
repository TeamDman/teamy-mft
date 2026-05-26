[CmdletBinding(PositionalBinding = $false)]
param(
	[switch]$Release,
	[switch]$NoOpenProfiler,
	[Parameter(Position = 0, ValueFromRemainingArguments = $true)]
	[string[]]$QueryArgs
)

$profilerScript = Join-Path $PSScriptRoot 'run-profiler.ps1'
$arguments = @()
if ($Release) {
	$arguments += '-Release'
}
if ($NoOpenProfiler) {
	$arguments += '-NoOpenProfiler'
}
$arguments += $QueryArgs

& $profilerScript @arguments
exit $LASTEXITCODE
