param(
    [string]$RepositoryUrl = "https://github.com/Burd-repo/benchmark",
    [string]$RunnerRoot = "C:\burd-actions-runner",
    [string]$RunnerName = "$env:COMPUTERNAME-burd-hardware",
    [string]$WorkFolder = "_work",
    [bool]$Ephemeral = $true,
    [switch]$Replace
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$token = $env:GITHUB_RUNNER_REGISTRATION_TOKEN
if ([string]::IsNullOrWhiteSpace($token)) {
    throw "Set GITHUB_RUNNER_REGISTRATION_TOKEN to a fresh repository runner registration token."
}

$root = [System.IO.Path]::GetFullPath($RunnerRoot)
if (Test-Path -LiteralPath (Join-Path $root ".runner")) {
    if (-not $Replace) {
        throw "A runner is already configured at $root. Use -Replace only when intentionally replacing it."
    }
} elseif (Test-Path -LiteralPath $root) {
    $existing = Get-ChildItem -LiteralPath $root -Force
    if ($existing.Count -gt 0) {
        throw "RunnerRoot must be empty before setup: $root"
    }
} else {
    New-Item -ItemType Directory -Path $root | Out-Null
}

$headers = @{
    Accept = "application/vnd.github+json"
    "User-Agent" = "burd-hardware-runner-bootstrap"
}
$release = Invoke-RestMethod `
    -Uri "https://api.github.com/repos/actions/runner/releases/latest" `
    -Headers $headers
$asset = $release.assets |
    Where-Object { $_.name -match "^actions-runner-win-x64-.*\.zip$" } |
    Select-Object -First 1
if ($null -eq $asset) {
    throw "Could not find the latest Windows x64 GitHub Actions runner archive."
}

$archive = Join-Path $root $asset.name
Write-Host "Downloading $($asset.name)..."
Invoke-WebRequest -Uri $asset.browser_download_url -Headers $headers -OutFile $archive
Expand-Archive -LiteralPath $archive -DestinationPath $root -Force
Remove-Item -LiteralPath $archive -Force

$config = Join-Path $root "config.cmd"
$arguments = @(
    "--unattended",
    "--url", $RepositoryUrl,
    "--token", $token,
    "--name", $RunnerName,
    "--labels", "burd-hardware",
    "--work", $WorkFolder
)
if ($Ephemeral) {
    $arguments += "--ephemeral"
}
if ($Replace) {
    $arguments += "--replace"
}

Write-Host "Registering runner $RunnerName with label burd-hardware..."
Push-Location $root
try {
    & $config @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "GitHub Actions runner configuration failed with exit code $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

Write-Host "Runner configured at $root."
Write-Host "Start it manually from a dedicated account only when dispatching the real-hardware workflow:"
Write-Host "  $root\run.cmd"
