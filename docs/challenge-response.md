# Challenge Response

The challenge-response flow prepares the future Burd backend validation path.

Flow:

1. Backend emits a `Challenge`.
2. Agent receives the challenge.
3. Agent runs the required local report/benchmark flow.
4. Agent hashes and signs the report.
5. Agent signs the challenge response payload:
   `challenge_id`, `nonce`, `provider_id`, `machine_id`, `report_hash`,
   `hardware_fingerprint`.
6. Backend can verify nonce, expiry, report hash, fingerprint, public key, and
   signature.

Current MVP commands:

```sh
burd-agent challenge run-local --json
burd-agent challenge create-mock --json
burd-agent challenge run --file docs/examples/challenge.json --json
burd-agent challenge verify --file signed-response.json --json
```

`run-local` creates and runs a local mock challenge in one command, then
persists the complete verified bundle as `latest-challenge-response.json`.
`create-mock` creates a local mock challenge. `run` accepts a challenge file and
returns the same bundle. `verify` expects that bundle so it can compare nonce,
expiry, required tests, minimum versions, report hash, and signatures.

Readiness revalidates the persisted bundle every time. Expired or invalid
evidence receives no Challenge points. A history entry containing a
`challenge_id` alone is not treated as verified evidence.

Challenge fields:

- `challenge_id`
- `nonce`
- `benchmark_profile`
- `required_tests`
- `issued_at`
- `expires_at`
- `is_expired`
- `age_seconds`
- `ttl_seconds`
- `backend_url`
- `min_agent_version`
- `min_benchmark_version`
- `policy.require_signed_report`
- `policy.require_llm_benchmark`
- `policy.require_stability`
- `policy.require_network`
- `policy.require_disk`

Challenge response fields:

- `challenge_id`
- `nonce`
- `provider_id`
- `machine_id`
- `report_hash`
- `hardware_fingerprint`
- `signed_report`
- `signature`
- `public_key`
- `completed_at`
- `issued_at`
- `expires_at`
- `is_expired`
- `age_seconds`
- `ttl_seconds`
- `status`: `passed`, `failed`, `expired`, or `partial`
- `failed_requirements`
- `verification_result`

Local validation:

- challenge must not be expired;
- response nonce must match;
- required tests must be present in the signed report;
- signed report hash must match the canonical report;
- signed report signature must verify when policy requires it;
- response fingerprint must match the signed report fingerprint;
- challenge response signature must verify;
- agent and benchmark versions must meet the challenge minimums.

Contract tests cover the local mock challenge flow without starting the API
server: a passing lightweight challenge, expired challenge rejection, required
test failures, wrong nonce rejection, response shape, signed report binding, and
hardware-fingerprint binding, and absence of private key/API token material in
the response.

Local challenges and their responses use a 24-hour TTL. Verification
recalculates freshness from the challenge issuance/expiry window rather than
trusting persisted `is_expired` or `age_seconds` values.

Backend future:

- production challenges;
- provider status updates;
- server-side fraud checks;
- backend challenge history;
- provider marketplace eligibility decisions.
