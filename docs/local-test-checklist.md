# Local Test Checklist

This checklist is for Windows PowerShell from the repository root. It uses the
debug binary path explicitly because `burd-agent` may not be installed on the
local `PATH` yet.

## Build And Smoke Tests

```powershell
cargo fmt
cargo test
cargo build
.\target\debug\burd-agent.exe --help
```

## Fast Local Commands

These commands do not start the local API server and do not run an infinite
heartbeat loop.

```powershell
.\target\debug\burd-agent.exe system --json
.\target\debug\burd-agent.exe fit --json --limit 3
.\target\debug\burd-agent.exe score --json
.\target\debug\burd-agent.exe identity init
.\target\debug\burd-agent.exe identity show --json
.\target\debug\burd-agent.exe report --json
.\target\debug\burd-agent.exe report --run-all --signed --json
.\target\debug\burd-agent.exe verify-provider --json
.\target\debug\burd-agent.exe challenge create-mock --json
.\target\debug\burd-agent.exe provider --json
.\target\debug\burd-agent.exe pricing --json
.\target\debug\burd-agent.exe earnings --json
.\target\debug\burd-agent.exe history --json
.\target\debug\burd-agent.exe history latest --json
.\target\debug\burd-agent.exe uptime --json
.\target\debug\burd-agent.exe actions --json
.\target\debug\burd-agent.exe logs --json
.\target\debug\burd-agent.exe registration-payload --json
.\target\debug\burd-agent.exe raw --json
```

Notes:

- `identity init` writes local public config to `~/.burd/agent.json` and the
  Ed25519 private key to `~/.burd/agent.key`.
- `report --run-all --signed --json` can run local benchmark modules and append
  benchmark history. Use it intentionally.
- `raw --json` may take longer than summary commands because it builds a
  redacted diagnostic payload. It must not expose private keys or API tokens.

## Ollama LLM Benchmark

Run this only when Ollama is installed and you want to execute a real local LLM
benchmark.

```powershell
ollama list
ollama pull llama3.2:1b
.\target\debug\burd-agent.exe bench llm --provider ollama --model llama3.2:1b --runs 1 --json
```

## Safe Scripted Checks

Use the local script for quick CLI validation:

```powershell
.\scripts\test-local.ps1
```

Use the API script to start the local server, test endpoints, and stop it by PID:

```powershell
.\scripts\test-api.ps1
```

