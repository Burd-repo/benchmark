# BN-19 - Observability And SRE

BN-19 adds the first production-oriented observability surface for the Burd
Control Plane. It is intentionally lightweight: the control plane can now expose
metrics, correlation IDs, structured logs, SLO status, and an admin operational
snapshot without introducing a vendor-specific telemetry stack.

## Scope

Implemented in BN-19:

- structured JSON logs for service start, HTTP requests, background task errors,
  and PostgreSQL connection errors;
- `x-burd-correlation-id` propagation for HTTP requests, with `x-request-id` as
  a fallback and generated request IDs when neither header is present;
- in-memory HTTP metrics for total requests, status classes, 5xx errors,
  rate-limited requests, in-flight requests, average latency, max latency, and
  recent p95 latency;
- background task error counters for session expiration and telemetry retention;
- Prometheus-compatible `GET /metrics` output;
- admin-protected `GET /v1/observability/snapshot` with HTTP, background, SLO,
  uptime, deployment, and recent event state;
- configurable deployment and SLO targets.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `BURD_CONTROL_DEPLOYMENT_ID` | `local` | Stable deployment label in logs, metrics, and snapshots. |
| `BURD_CONTROL_OBSERVABILITY_RECENT_EVENTS_LIMIT` | `100` | Number of recent normalized HTTP events kept in memory. |
| `BURD_CONTROL_SLO_AVAILABILITY_TARGET_BPS` | `9990` | Availability target in basis points, where `9990` means 99.90%. |
| `BURD_CONTROL_SLO_P95_LATENCY_MS` | `500` | Recent p95 HTTP latency target in milliseconds. |

## API

### `GET /metrics`

Returns text/plain Prometheus metrics. The current implementation exports
process uptime, HTTP request counters, 5xx errors, in-flight requests, average
latency, recent p95 latency, availability ratio, and background task errors.

This endpoint contains aggregate operational data only. It does not expose raw
payloads, credentials, tokens, provider private material, Pix keys, or customer
workload bytes.

### `GET /v1/observability/snapshot`

Admin-only endpoint protected by `Authorization: Bearer <admin-token>`.

The response includes:

- service, environment, deployment ID, start time, and uptime;
- HTTP totals and status classes;
- recent normalized HTTP events with correlation IDs;
- background task error totals;
- SLO targets and current status.

Recent event paths are normalized to avoid high-cardinality identifiers in the
snapshot and logs.

## Operational Runbook

Basic health check:

```bash
curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/ready
```

Metrics check:

```bash
curl http://127.0.0.1:8080/metrics
```

Admin snapshot:

```bash
curl -H "Authorization: Bearer $BURD_CONTROL_ADMIN_TOKEN" \
  http://127.0.0.1:8080/v1/observability/snapshot
```

Incident triage order:

1. Check `/ready` for database or migration failure.
2. Check `/metrics` for 5xx, in-flight request growth, p95 latency, and
   background task errors.
3. Check `/v1/observability/snapshot` for recent correlated requests.
4. Query audit events for state-changing backend decisions related to the same
   entity or request window.
5. Verify PostgreSQL backups and object-storage envelope availability before
   destructive operator action.

## Backup And Restore Contract

BN-19 documents the operational expectation but does not automate backup jobs.
Production deployment must provide:

- PostgreSQL backups with restore tests;
- object-storage backups for evidence, proof, and benchmark envelopes;
- retention policy aligned with audit and billing requirements;
- restore rehearsal before production marketplace launch.

## Non-Goals

BN-19 does not implement:

- OpenTelemetry Collector export;
- dashboards-as-code;
- alert manager routing;
- automated backup scheduling;
- automated restore tooling;
- incident ticketing integration;
- distributed tracing across separate services.

Those belong to the production hardening path once the deployment environment is
known.