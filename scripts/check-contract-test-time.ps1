param(
    [double]$MaxSeconds = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

Write-Host "Running fast burd-bench contract tests with a ${MaxSeconds}s budget..."
$elapsed = Measure-Command {
    & cargo test -p burd-bench --lib --quiet
    if ($LASTEXITCODE -ne 0) {
        throw "burd-bench contract tests failed with exit code $LASTEXITCODE"
    }
}

$seconds = [Math]::Round($elapsed.TotalSeconds, 2)
Write-Host "Fast burd-bench contract tests completed in ${seconds}s."

if ($elapsed.TotalSeconds -gt $MaxSeconds) {
    throw "Fast contract test budget exceeded: ${seconds}s > ${MaxSeconds}s"
}
