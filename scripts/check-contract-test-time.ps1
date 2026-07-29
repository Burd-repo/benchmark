param(
    [double]$MaxSeconds = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

Write-Host "Running fast burd-bench contract tests with a ${MaxSeconds}s budget..."
$testExitCode = 1
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
try {
    & cargo test -p burd-bench --lib --quiet
    $testExitCode = $LASTEXITCODE
}
finally {
    $stopwatch.Stop()
}

if ($testExitCode -ne 0) {
    throw "burd-bench contract tests failed with exit code $testExitCode"
}

$seconds = [Math]::Round($stopwatch.Elapsed.TotalSeconds, 2)
Write-Host "Fast burd-bench contract tests completed in ${seconds}s."

if ($stopwatch.Elapsed.TotalSeconds -gt $MaxSeconds) {
    throw "Fast contract test budget exceeded: ${seconds}s > ${MaxSeconds}s"
}
