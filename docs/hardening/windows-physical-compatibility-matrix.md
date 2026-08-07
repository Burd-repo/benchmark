# Windows Physical Compatibility Matrix

Date: 2026-07-27

## Summary

This pass validates the real Burd Agent binary against the available Windows
hardware, local Ollama service, Docker Desktop, signed local state, challenge
flow, readiness, workload eligibility, registration payload, raw redaction, and
local HTTP API.

No product defect was reproduced. No Rust code, protocol, migration, dependency,
or product feature changed.

The tested host has an AMD GPU. This matrix does not validate physical NVIDIA
hardware or CUDA. Those paths remain unverified until they run on a controlled
NVIDIA/CUDA host.

## Sanitization

The report intentionally omits:

- provider and machine IDs;
- hardware fingerprint;
- GPU UUID;
- public and private keys;
- signatures and report hashes;
- API bearer tokens and token hashes;
- user-specific local filesystem paths;
- complete model digests.

## Environment

| Component | Observed environment | Result |
| --- | --- | --- |
| OS | Windows, `x86_64` | Detected |
| GPU | AMD Radeon RX 5700 XT, 8 GB | Detected |
| GPU backend | Vulkan | Detected |
| VRAM evidence | `vulkan_device_memory`, confidence `detected` | Passed |
| NVIDIA tooling | `nvidia-smi` unavailable | Not testable |
| CUDA toolkit | `nvcc` and CUDA Toolkit unavailable | Not testable |
| ROCm | Unavailable | Not testable |
| Ollama | `0.32.4`, three installed models | Available |
| Docker Desktop | `4.70.0` | Available |
| Docker Engine | `29.4.0`, Linux/WSL2 | Available |

Windows reports the AMD adapter as healthy. The Agent leaves `amd_driver`
absent because its current AMD driver probe depends on `rocm-smi`, which is not
available on this host. This does not affect the NVIDIA/CUDA-only marketplace
MVP gate.

## Agent Contracts

### Hardware And Marketplace Policy

`burd-agent system --json` detected:

- one AMD GPU;
- 8 GB VRAM from Vulkan device memory;
- `backend_detected=Vulkan`;
- `cuda_available=false`;
- `rocm_available=false`.

`burd-agent fingerprint --json` preserved:

- `burd-hardware-fingerprint-v1`;
- `marketplace_eligible=false`;
- `eligibility_level=local_diagnostic_only`;
- `gpu_policy=nvidia_cuda_only_mvp`.

The observed Compute Score remained separate from marketplace policy.
`score.eligible=true` represented only the local score threshold; it did not
override `marketplace_eligible=false`.

### Live Ollama

The existing ignored compatibility test passed against the running local Ollama
service and a real installed model. It validated model inventory discovery,
prefixed `sha256:` artifact binding, short inference, finite tokens per second,
and positive TTFT.

A public CLI benchmark using `llama3.2:1b` and three runs passed with no warnings
or errors. One observed run produced:

- average throughput: 71.45 tokens/s;
- minimum throughput: 70.44 tokens/s;
- maximum throughput: 72.55 tokens/s;
- average TTFT: 35.9 ms;
- average latency: 2531.3 ms.

These values are a point-in-time local observation. They are not a product
baseline, SLA, remote attestation, scheduler decision, or marketplace promise.

### Signed Report And History

`report --run-all --signed` completed against Ollama and produced:

- a locally valid Ed25519 signature;
- `burd-json-c14n-v1` canonicalization;
- fresh evidence with a seven-day local TTL;
- passing LLM, network, and disk sections;
- one persisted signed history entry;
- `verification_status=signature_valid_locally`.

`verify-report` accepted the saved envelope with no errors or warnings. The
report remained local evidence, not backend-attested evidence.

### Local Challenge

`challenge run-local --profile profile_8gb` passed:

- response nonce matched the challenge nonce;
- response signature was present and valid;
- evidence was fresh;
- no required test failed;
- local verification returned valid.

The command and output remain explicitly local/mock. This is not Burd remote
Proof of Capability.

### Readiness And Eligibility

With isolated identity, signed report, history, challenge, and local API token,
readiness reached:

- `status=ready_locally`;
- score `100`;
- all seven local checks passed;
- no readiness warnings.

This did not make the host marketplace eligible. Workload eligibility reported:

- provider tier `Burd Plus` from local Compute Score;
- aggregate local status `diagnostic_only`;
- nine workloads `diagnostic_only`;
- three workloads `not_recommended`;
- all twelve workloads `marketplace_blocked`.

Capability Spot reported `verified_locally` with
`verification_mode=local_mock`, not remote verification.

### Registration And Redaction

The registration payload preserved:

- `marketplace_eligible=false`;
- `eligibility_level=local_diagnostic_only`;
- backend `Vulkan`;
- detected VRAM source and confidence;
- local/mock capability status;
- future marketplace status `marketplace_blocked`;
- `secrets_included=false`.

The raw endpoint, registration payload, local API responses, and server logs did
not contain the captured private key or API bearer token.

### Local HTTP API

A temporary server bound to `127.0.0.1:8791` returned:

- `GET /health`: HTTP `200`;
- unauthenticated `GET /api/v1/config`: HTTP `401`;
- authenticated `GET /api/v1/config`: HTTP `200`;
- `GET /api/v1/system`: HTTP `200`.

The authenticated response redacted the token hash and private key path. The
captured bearer token did not appear in response bodies or server logs.

### Secure Runtime

> Historical note: this capture exercised the v1 runtime model. Runtime
> Platform Model v2 supersedes the global `unsupported_host` interpretation by
> separating a Windows host from its future `docker_wsl2` Linux-container
> backend. The observations below remain the literal result of this earlier
> physical run.

`runtime check --json` observed Docker Engine `29.4.0` and an advertised NVIDIA
container runtime, but correctly returned `status=unsupported_host` because:

- BN-12 runtime support is Linux-first;
- the host is Windows;
- no physical NVIDIA GPU UUID was available;
- no signed image reference was supplied.

Docker availability did not override the host or GPU requirements.

## Commands Executed

Core environmental tests:

```powershell
cargo test -p burd-agent remote_proof::ollama::tests::live_ollama_inference_binds_prefixed_inventory_digest -- --ignored --exact --nocapture
cargo test -p burd-bench contract_tests::real_hardware_detection_integration_is_available -- --ignored --exact --nocapture
```

Representative Agent commands used with an isolated
`BURD_AGENT_CONFIG` under `C:\tmp`:

```powershell
burd-agent system --json
burd-agent fingerprint --json
burd-agent bench llm --provider ollama --model llama3.2:1b --runs 3 --json
burd-agent report --run-all --signed --provider ollama --model llama3.2:1b --json
burd-agent verify-report --file <isolated-report> --json
burd-agent challenge run-local --profile profile_8gb --json
burd-agent readiness --json
burd-agent capability-spot --json
burd-agent workload-eligibility --json
burd-agent registration-payload --json
burd-agent raw --json
burd-agent runtime check --json
burd-agent serve --host 127.0.0.1 --port 8791
```

## Repository Validation

- `cargo fmt --all --check`: passed.
- `cargo test --workspace`: 266 passed and 25 ignored by default.
- Both environment-dependent ignored tests passed separately: live Ollama and
  physical hardware detection.
- The remaining 23 ignored tests passed against isolated PostgreSQL: 20 Control
  Plane tests and 3 Agent/WebSocket tests.
- `cargo build --workspace`: passed.
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings`: failed on
  pre-existing lints in `burd-bench` and Control Plane billing. Findings include
  `too_many_arguments`, `needless_borrow`, `needless_return`, `collapsible_if`,
  and `collapsible_str_replace`.

The Clippy findings did not originate from this documentation-only branch and
were not mixed into the physical compatibility PR. They should be handled in a
separate, focused cleanup with proportional tests.
## Transient Harness Issues

- Restricted sandbox access initially prevented direct Docker named-pipe access
  and a first runtime check. Both probes were repeated with approved local
  access and passed.
- Restricted sandbox access initially prevented Ollama log rotation and service
  startup. The service was then confirmed through its local API as version
  `0.32.4` with three installed models, and the live test passed.
- Restricted sandbox access initially prevented disposable identity and
  challenge persistence under `C:\tmp`. The same operations passed with
  approved write access.
- A first Windows CIM query was denied by the sandbox. The approved query
  detected the AMD adapter with healthy Windows status.
- Two initial HTTP inspection scripts completed the server requests but failed
  while formatting an empty log value. The final deterministic `curl.exe` run
  passed all HTTP and redaction assertions.
- Initial command discovery tried unsupported aliases for history, challenge,
  runtime, and API token commands. Clap rejected them and the documented
  subcommands were then used.
- An initial PowerShell inventory pipeline had invalid syntax. A separate
  environment enumeration also hit a duplicate-key error; command/path probes
  and Agent output independently confirmed CUDA absence.

These were harness or sandbox failures, not Burd Agent failures.

## Remaining Limits

- No physical NVIDIA GPU or CUDA runtime was available.
- No Linux host with NVIDIA Container Toolkit and a bound GPU UUID was tested.
- No long soak or thermal-throttling matrix was executed.
- Only one installed Ollama model was used for the measured CLI benchmark.
- Local evidence does not replace backend-issued challenge verification.
- Existing `third_party/llmfit` dead-code warnings remain unchanged.

## Conclusion

The available AMD/Windows environment behaves as intended:

- useful local diagnostics and real Ollama benchmarking remain available;
- signed local evidence and local challenge verification work;
- readiness can become `ready_locally`;
- marketplace and future workload admission remain blocked;
- under the captured v1 model, secure runtime was reported as unsupported on
  the Windows host; v2 now models Windows as a potential `docker_wsl2` host but
  keeps it `not_ready` until backend and physical NVIDIA verification exist;
- secrets remain redacted.

Physical NVIDIA/CUDA certification requires a separate controlled host matrix.
