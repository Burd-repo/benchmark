Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

$previous = $env:BURD_UPDATE_CONTRACT_SNAPSHOTS
$env:BURD_UPDATE_CONTRACT_SNAPSHOTS = "1"
try {
    & cargo test -p burd-bench contract_tests::sanitized_json_contract_snapshots_are_stable -- --exact
    if ($LASTEXITCODE -ne 0) {
        throw "Contract snapshot update failed with exit code $LASTEXITCODE"
    }
} finally {
    if ($null -eq $previous) {
        Remove-Item Env:\BURD_UPDATE_CONTRACT_SNAPSHOTS -ErrorAction SilentlyContinue
    } else {
        $env:BURD_UPDATE_CONTRACT_SNAPSHOTS = $previous
    }
}

& cargo test -p burd-bench contract_tests::sanitized_json_contract_snapshots_are_stable -- --exact
if ($LASTEXITCODE -ne 0) {
    throw "Updated contract snapshot verification failed with exit code $LASTEXITCODE"
}
