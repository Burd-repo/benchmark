Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

$OutputDir = Join-Path $RepoRoot "tmp\test-output"
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

$Agent = Join-Path $RepoRoot "target\debug\burd-agent.exe"

function Invoke-External {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $outFile = Join-Path $OutputDir "$Name.txt"
    Write-Host "==> $Name"
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $Command @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    $outputText = if ($null -eq $output) { "" } else { $output | Out-String }

    $outputText | Set-Content -Path $outFile

    if ($exitCode -ne 0) {
        Write-Host $outputText
        throw "$Name failed with exit code $exitCode. See $outFile"
    }

    Write-Host "    saved $outFile"
}

try {
    Invoke-External "cargo-fmt" "cargo" @("fmt")
    Invoke-External "cargo-test" "cargo" @("test")
    Invoke-External "cargo-build" "cargo" @("build")

    if (-not (Test-Path $Agent)) {
        throw "Missing agent binary: $Agent. cargo build did not produce the expected executable."
    }

    Invoke-External "burd-help" $Agent @("--help")
    Invoke-External "system" $Agent @("system", "--json")
    Invoke-External "fit-limit-3" $Agent @("fit", "--json", "--limit", "3")
    Invoke-External "score" $Agent @("score", "--json")
    Invoke-External "provider" $Agent @("provider", "--json")
    Invoke-External "verify-provider" $Agent @("verify-provider", "--json")
    Invoke-External "pricing" $Agent @("pricing", "--json")
    Invoke-External "earnings" $Agent @("earnings", "--json")
    Invoke-External "raw" $Agent @("raw", "--json")

    Write-Host "Local test completed. Outputs saved in $OutputDir"
} catch {
    Write-Error "Local test failed: $($_.Exception.Message)"
    exit 1
}
