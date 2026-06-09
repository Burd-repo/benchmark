param(
    [string]$RunnerRoot = "C:\burd-actions-runner"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$token = $env:GITHUB_RUNNER_REMOVE_TOKEN
if ([string]::IsNullOrWhiteSpace($token)) {
    throw "Set GITHUB_RUNNER_REMOVE_TOKEN to a fresh repository runner removal token."
}

$root = [System.IO.Path]::GetFullPath($RunnerRoot)
$config = Join-Path $root "config.cmd"
if (-not (Test-Path -LiteralPath $config)) {
    throw "Runner config command not found at $config"
}

Push-Location $root
try {
    & $config remove --token $token
    if ($LASTEXITCODE -ne 0) {
        throw "GitHub Actions runner removal failed with exit code $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

Write-Host "Runner registration removed. Runner files remain at $root for manual inspection/removal."
