# AI Performance Metrics

AI Performance Metrics consolidates local AI performance signals into one stable report for CLI, local API, provider details, raw data, registration payloads, full reports, and signed reports.

It does not run a benchmark automatically. It does not start Ollama, vLLM, MLX, a server, a browser, a daemon, or any remote runtime. It does not create backend verification, remote Proof of Capability, marketplace approval, scheduler admission, jobs, leases, billing, Pix, or payouts.

## Commands and API

```sh
burd-agent ai-performance --json
```

```txt
GET /api/v1/ai-performance
```

Both surfaces reuse the same local report builder.

## Status

- `measured`: a current measured LLM benchmark is available from the active report flow, a valid signed report, or benchmark history.
- `estimated`: no measured benchmark is available, but fit analysis has a runnable model estimate.
- `expired`: measured benchmark evidence exists, but the evidence TTL is expired.
- `partial`: measured benchmark evidence exists but is degraded, failed, or incomplete.
- `not_measured`: neither measured benchmark evidence nor a runnable fit estimate is available.
- `invalid`: reserved for future validation failures.

## Sources

- `real_benchmark`: LLM benchmark data from the current report generation flow.
- `signed_report`: LLM benchmark data from the latest signed report.
- `benchmark_history`: LLM benchmark summary from local benchmark history.
- `fit_estimate`: llmfit model-fit estimate. This is never treated as measured proof.
- `not_measured`: no reliable data is available.

Signed reports use the local signed-report TTL. Expired signed reports can still be displayed for audit/history, but the report marks `is_expired: true`, lowers confidence, and adds a warning.

## Metrics

Measured when available:

- `tokens_per_second` from benchmark `avg_tps`.
- `sustained_tokens_per_second` from benchmark `min_tps`.
- `time_to_first_token_ms` from benchmark `avg_ttft_ms`.
- `latency_p50_ms` and `latency_p95_ms` from per-run latency details when present.
- `benchmark_runs`, `model`, `provider`, and `runtime` from the LLM benchmark payload.

Explicitly nullable when not measured:

- `requests_per_second`.
- `latency_p95_ms` when per-run latency is not present.
- `time_to_first_token_ms` when the runtime did not provide TTFT.
- future image, transcription, embedding, and training metrics.

Hardware and compatibility context comes from the existing system report and fit analysis: backend, driver, GPU name, VRAM, compatible models, limited models, and max recommended model class.

## Confidence

- `high`: current real benchmark data or valid current signed evidence.
- `medium`: benchmark history or signed evidence with reduced trust.
- `low`: fit estimate only.
- `unavailable`: metric not measured.

Local trust score, workload eligibility, and capability spot are local signals only. They are not remote confidence, remote Proof of Capability, or marketplace approval.
