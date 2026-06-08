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
`verify` currently expects that same bundle so it can compare nonce and expiry.

Backend future:

- production challenges;
- provider status updates;
- server-side fraud checks;
- backend challenge history;
- provider marketplace eligibility decisions.

