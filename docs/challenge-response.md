# Challenge Response

The challenge-response flow prepares the future Burd backend validation path.

Flow:

1. Backend emits a `Challenge`.
2. Agent receives the challenge.
3. Agent runs the required local report/benchmark flow.
4. Agent hashes and signs the report.
5. Agent signs the challenge response payload:
   `challenge_id`, `nonce`, `provider_id`, `machine_id`, `report_hash`.
6. Backend can verify nonce, expiry, report hash, public key, and signature.

Current MVP commands:

```sh
burd-agent challenge create-mock --json
burd-agent challenge run --file docs/examples/challenge.json --json
burd-agent challenge verify --file signed-response.json --json
```

`create-mock` creates a local mock challenge. `run` returns a bundle containing
the original challenge, signed report, response, and local verification result.
`verify` expects that same bundle so it can compare nonce, expiry, required
tests, minimum versions, report hash, and signatures.

Challenge fields:

- `challenge_id`
- `nonce`
- `benchmark_profile`
- `required_tests`
- `issued_at`
- `expires_at`
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
- `signed_report`
- `signature`
- `public_key`
- `completed_at`
- `status`: `passed`, `failed`, `expired`, or `partial`
- `failed_requirements`
- `verification_result`

Local validation:

- challenge must not be expired;
- response nonce must match;
- required tests must be present in the signed report;
- signed report hash must match the canonical report;
- signed report signature must verify when policy requires it;
- challenge response signature must verify;
- agent and benchmark versions must meet the challenge minimums.

Contract tests cover the local mock challenge flow without starting the API
server: a passing lightweight challenge, expired challenge rejection, required
test failures, wrong nonce rejection, response shape, signed report binding, and
absence of private key/API token material in the response.

Backend future:

- production challenges;
- provider status updates;
- server-side fraud checks;
- backend challenge history;
- provider marketplace eligibility decisions.
