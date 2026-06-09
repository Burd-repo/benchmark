Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

$OutputDir = Join-Path $RepoRoot "tmp\test-output"
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

$Agent = Join-Path $RepoRoot "target\debug\burd-agent.exe"
$HostAddress = "127.0.0.1"
$Port = 8787
$BaseUrl = "http://$HostAddress`:$Port"
$EndpointTimeoutSec = 45
$ServerOut = Join-Path $OutputDir "api-server.out.txt"
$ServerErr = Join-Path $OutputDir "api-server.err.txt"
$serverProcess = $null

function Write-EndpointOutput {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Content
    )

    $path = Join-Path $OutputDir "api-$Name.json"
    $Content | Set-Content -Path $path
}

function Invoke-Endpoint {
    param(
        [Parameter(Mandatory = $true)][string]$Path
    )

    $name = $Path.Trim("/").Replace("/", "-")
    if ([string]::IsNullOrWhiteSpace($name)) {
        $name = "root"
    }

    $uri = "$BaseUrl$Path"
    Write-Host "==> GET $Path"
    try {
        $response = Invoke-WebRequest -Uri $uri -UseBasicParsing -TimeoutSec $EndpointTimeoutSec
        Write-EndpointOutput $name $response.Content
        Write-Host "    $($response.StatusCode)"
    } catch {
        $response = $_.Exception.Response
        $statusCode = $null
        if ($null -ne $response) {
            $statusCode = [int]$response.StatusCode
        }

        if ($statusCode -eq 401) {
            $message = @{
                status = "token_required"
                endpoint = $Path
                note = "Endpoint requires Authorization: Bearer <token> when local API auth is enabled."
            } | ConvertTo-Json -Depth 4
            Write-EndpointOutput $name $message
            Write-Host "    401 token required"
            return
        }

        throw "GET $Path failed: $($_.Exception.Message)"
    }
}

function Save-ServerOutput {
    if ($null -eq $serverProcess) {
        return
    }

    if (-not $serverProcess.HasExited) {
        return
    }

    try {
        $serverProcess.StandardOutput.ReadToEnd() | Set-Content -Path $ServerOut
    } catch {
        "failed to read stdout: $($_.Exception.Message)" | Set-Content -Path $ServerOut
    }

    try {
        $serverProcess.StandardError.ReadToEnd() | Set-Content -Path $ServerErr
    } catch {
        "failed to read stderr: $($_.Exception.Message)" | Set-Content -Path $ServerErr
    }
}

try {
    if (-not (Test-Path $Agent)) {
        throw "Missing agent binary: $Agent. Run cargo build first."
    }

    "" | Set-Content -Path $ServerOut
    "" | Set-Content -Path $ServerErr

    Write-Host "==> Starting local API on $BaseUrl"
    $processInfo = New-Object System.Diagnostics.ProcessStartInfo
    $processInfo.FileName = $Agent
    $processInfo.Arguments = "serve --host $HostAddress --port $Port"
    $processInfo.WorkingDirectory = $RepoRoot.Path
    $processInfo.UseShellExecute = $false
    $processInfo.CreateNoWindow = $true
    $processInfo.RedirectStandardOutput = $true
    $processInfo.RedirectStandardError = $true

    $serverProcess = New-Object System.Diagnostics.Process
    $serverProcess.StartInfo = $processInfo
    $serverProcess.Start() | Out-Null

    $healthy = $false
    for ($i = 0; $i -lt 10; $i++) {
        Start-Sleep -Milliseconds 500
        if ($serverProcess.HasExited) {
            break
        }

        try {
            $response = Invoke-WebRequest -Uri "$BaseUrl/health" -UseBasicParsing -TimeoutSec 2
            if ($response.StatusCode -eq 200) {
                $healthy = $true
                break
            }
        } catch {
            $healthy = $false
        }
    }

    if (-not $healthy) {
        throw "Local API did not become healthy within 5 seconds. Check $ServerOut and $ServerErr."
    }

    Invoke-Endpoint "/health"
    Invoke-Endpoint "/api/v1/system"
    Invoke-Endpoint "/api/v1/score"
    Invoke-Endpoint "/api/v1/provider"
    Invoke-Endpoint "/api/v1/readiness"
    Invoke-Endpoint "/api/v1/verification"
    Invoke-Endpoint "/api/v1/pricing"
    Invoke-Endpoint "/api/v1/earnings"
    Invoke-Endpoint "/api/v1/raw"

    Write-Host "API test completed. Outputs saved in $OutputDir"
} catch {
    Write-Error "API test failed: $($_.Exception.Message)"
    exit 1
} finally {
    if ($null -ne $serverProcess -and -not $serverProcess.HasExited) {
        Write-Host "==> Stopping local API PID $($serverProcess.Id)"
        Stop-Process -Id $serverProcess.Id -Force -ErrorAction SilentlyContinue
        $serverProcess.WaitForExit(5000) | Out-Null
    }
    Save-ServerOutput
    if ($null -ne $serverProcess) {
        $serverProcess.Dispose()
    }
}
