# Live Telemetry To Proof Linkage

## Summary

This hardening pass closes a BN-04/BN-06 trust gap: a Proof of Capability response that claims `telemetry_window` must now reference a telemetry batch the backend already accepted for the same provider, device, session, hardware fingerprint, and proof GPU UUID.

No agent daemon, proof workload runner, scheduler loop, job runtime, marketplace feature, Pix gateway, billing automation, or payout flow was added.

## Scope Audited

- `crates/burd-control-plane/src/proof_challenge.rs`
- `crates/burd-control-plane/src/telemetry.rs`
- `crates/burd-control-plane/src/http.rs`
- BN-04 signed telemetry batch registry and samples
- BN-06 proof challenge response verification
- Existing live provider enrollment/session/proof fixtures

## Implemented For Real

- Proof verification now checks required `telemetry_window_hash` values against `telemetry_batches.batch_hash`.
- The linked telemetry batch must match provider, device, session, hardware fingerprint, and proof GPU UUID.
- The live proof test now submits a signed BN-04 telemetry batch before submitting the BN-06 proof response.
- A negative live test verifies a syntactically valid but unregistered telemetry hash fails the proof response.

## Still Planned, Mock, Or Local

- The signed telemetry payload is a deterministic test fixture, not a real `nvidia-smi` collection.
- The proof metrics are deterministic test values, not a real CUDA/VRAM/GEMM/LLM workload execution.
- The agent still does not automatically capture a telemetry window during proof execution.
- WebSocket telemetry streaming is not expanded here; the live test uses the authenticated HTTP telemetry fallback.

## Bugs Found

- BN-06 previously treated `telemetry_window_hash` as a bounded string when `telemetry_window` was required. It did not verify that the hash referred to a server-accepted BN-04 telemetry batch.

## Bugs Fixed

- Required proof telemetry windows now fail verification unless the hash exists in the backend telemetry registry for the same provider/device/session/fingerprint.
- Required proof telemetry windows now fail verification when the referenced batch does not include the proof GPU UUID.
- Proof responses with invalid telemetry linkage set `metrics_satisfied=false` and persist as failed challenges.

## Overengineering Removed

- No production abstraction was added.
- Test setup reuses the existing live enrollment/session fixture and adds focused telemetry/proof helpers.

## Events And Listeners

- No event bus, listener, async dispatch, background event, or outbox behavior changed.

## Migrations And Database

- No migrations were added or edited.
- The fix uses existing `telemetry_batches.batch_hash`, provider/device/session/fingerprint fields, and `gpu_telemetry_samples.gpu_uuid` rows.

## Security Findings

- The proof response can no longer self-claim an arbitrary telemetry window hash.
- The linked telemetry batch must have passed BN-04 signature, hash, sequence, fingerprint, and sample validation before BN-06 can accept it.
- No private key, bearer credential, resume token, admin token, or database URL was committed.

## Performance Findings

- The new verification query runs only when `telemetry_window` is required.
- The query is bounded by the unique telemetry batch hash and existing sample rows for that batch.
- The live tests remain ignored by default and require an explicit PostgreSQL test database.

## Tests Added

- `http::tests::live_proof_challenge_rejects_unregistered_telemetry_window_hash`

## Tests Changed

- `http::tests::live_proof_challenge_http_flow_verifies_signed_response` now submits a signed telemetry batch and uses its accepted `batch_hash` as the proof `telemetry_window_hash`.

## Tests Executed

- `cargo test -p burd-control-plane` passed: 100 passed, 19 ignored.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane live_proof_challenge_http_flow_verifies_signed_response -- --ignored` passed: 1 passed.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane live_proof_challenge_rejects_unregistered_telemetry_window_hash -- --ignored` passed: 1 passed.
- `BURD_CONTROL_TEST_DATABASE_URL=postgres://burd:burd@127.0.0.1:5432/burd_test cargo test -p burd-control-plane -- --ignored` passed: 19 passed.
- `cargo fmt --all --check` passed.
- `cargo test --workspace` passed.
- `cargo build --workspace` passed.

## Tests Not Executed Yet In This Pass

- Real agent telemetry collection.
- Real CUDA/VRAM/GEMM/LLM proof execution.
- Scheduler, marketplace, billing, Pix, payout, and secure runtime end-to-end flows.

## Remaining Risks

- A backend-accepted telemetry batch proves the reported signed telemetry existed, but this pass still does not prove agent-side timing overlap with a real proof workload.
- The live helper cleans temporary schemas only on successful test completion; a panic can leave disposable test schemas behind.
- Automatic challenge telemetry capture remains a future agent/runtime concern.

## Deferred Items

- Add agent-daemon proof execution coverage once the daemon and runner exist.
- Add WebSocket telemetry streaming coverage if the control-channel path needs separate live exercise beyond HTTP fallback.
- Consider explicit proof execution window overlap checks once telemetry capture is orchestrated by the agent.

## Recommended Next Hardening PRs

- Harden agent-side remote session retry/backoff state once daemon work starts.
- Add proof telemetry timing-window checks after agent-side capture exists.
- Split live control-plane fixtures out of `http.rs` if more live tests reuse them.