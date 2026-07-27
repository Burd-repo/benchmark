# Agent Cooperative Proof Shutdown

## Summary

The foreground Agent now propagates its existing shutdown signal into an active
Proof of Capability execution. This hardening does not add service packaging,
an Agent daemon, a new proof profile, or a protocol change.

## Behavior

- The proof supervisor observes shutdown while waiting for executor readiness,
  telemetry delivery, telemetry capture, and proof completion.
- A shutdown requests cancellation, releases a proof blocked at the telemetry
  residency gate, and gives the executor five seconds to stop cooperatively.
- An operator-requested shutdown does not submit a partial response and is not
  recorded as a failed proof attempt.
- CUDA execution checks cancellation around library validation, NVIDIA
  telemetry, VRAM residency, each SGEMM iteration, and LLM inference.
- Ollama execution checks cancellation before and after inventory requests,
  after the generation request, and between streamed response lines.
- Response submission retains its existing bounded HTTP request once a complete,
  signed response has been produced. Cancelling it locally would leave an
  ambiguous submission result.

## Security And Consistency

Cancellation state is process-local and is not included in signed protocol
payloads. It cannot change challenge inputs, metrics, evidence hashes, or
signatures. A response is still created only after the required telemetry window
and proof execution both complete.

Challenge identifiers may appear in the structured cancellation deadline event.
Credentials, tokens, private keys, response payloads, and authorization headers
are not logged.

## Validation

The focused unit test
`active_proof_execution_stops_cooperatively_on_shutdown` drives an executor
through the telemetry gate, requests shutdown, and verifies that the operation
returns without a response or failed-attempt result.

Validation commands for this change:

```powershell
cargo test -p burd-agent --lib active_proof_execution_stops_cooperatively_on_shutdown
cargo test -p burd-agent --lib
```

The final branch validation and its results are recorded in the pull request.

## Remaining Limitations

- CUDA driver calls, synchronization calls, and an in-progress blocking Ollama
  HTTP read cannot be force-cancelled safely. They observe cancellation at the
  next checkpoint or their existing timeout.
- Aborting a Tokio blocking-task handle after the five-second grace period does
  not terminate native work that has already started. The grace period bounds
  supervisor waiting, not every vendor runtime call or total process exit time.
- Startup hardware registration and credential requests do not yet share this
  cancellation path.
- The Agent remains a foreground command. Windows Service/systemd packaging,
  stable service exit categories, signed updates, and rollback remain separate
  work.
