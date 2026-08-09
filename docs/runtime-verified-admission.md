# Runtime-Verified Provider Admission

This slice derives one fail-closed `RuntimeAdmissionDecision` per provider/device/GPU. The
admin-triggered scheduler now consumes that decision transactionally, while marketplace, billing
and the deliberately disabled production Provider Job Worker remain unchanged.

## Authority model

The Control Plane combines three different sources rather than trusting any one of them alone:

```text
current signed GPU inventory
            +
TTL-bound runtime verification proof
            +
recent signed runtime observation
            |
            v
derived runtime admission
```

The challenge is session-bound. The resulting verification is device/GPU/runtime-bound and can
survive a reconnect when the provider, device, active key, hardware fingerprint, GPU UUID and
runtime fingerprint remain unchanged. Because admission binds the proof, observation and latest
signed GPU inventory to the active device key, recovery after rotation or revocation requires the
Agent to republish all three artifacts with the new key, in order: GPU inventory, runtime
observation and runtime verification proof.

`runtime_verified` means that Burd accepted a cryptographically bound functional proof and that the
currently observed runtime still matches it. It does not mean `hardware_attested`: the provider
controls the host OS, Agent executable and local signing key.

## Current observation

The supervised Agent worker probes the already implemented Docker backend every 60 seconds and
submits `POST /v1/sessions/{session_id}/runtime-observations`. The payload binds:

- provider, device, current session and hardware fingerprint;
- host OS and runtime backend;
- Linux container, CUDA, NVIDIA and isolation contracts;
- Docker server, NVIDIA driver and NVIDIA runtime versions;
- the complete currently observed GPU UUID set;
- Agent runtime contract version and observation time.

The payload is canonicalized, hashed and signed with the active Ed25519 device key. The Control
Plane independently checks session identity/status, provider/device state, hardware fingerprint,
active signer key, signature, freshness and exact agreement with the latest signed active GPU
inventory before persisting the immutable observation. No bearer credential, session token or
private key enters the payload or audit metadata.

`BURD_CONTROL_RUNTIME_OBSERVATION_MAX_AGE_SECONDS` controls freshness and is bounded to 60-3600
seconds; the default is 180 seconds. Failure to produce a fresh observation denies admission.

## Admission evaluation

`GET /v1/providers/{provider_id}/runtime-admissions` is admin-only and evaluates admission from
current persisted evidence. The decision is not cached or self-reported by the provider.

Admission requires:

- provider not blocked/quarantined and device/GPU active;
- exactly one active device key, matching inventory, observation and proof signer;
- recent observation from an online/degraded current session;
- unchanged session/observation hardware fingerprint;
- challenged GPU still present in the observation;
- v2 verification record with `verified` status and unexpired TTL;
- unchanged GPU UUID, runtime backend and admission fingerprint;
- proof image equal to the Control Plane-owned digest in
  `BURD_CONTROL_RUNTIME_PROOF_IMAGE_REF`.

Driver, Docker, hardware, GPU or backend drift changes the admission fingerprint and produces
`denied`. A session reconnect alone does not. Reason codes are stable and sorted for operator use.
Key rotation remains fail-closed until a new signed GPU inventory, signed runtime observation and
runtime verification proof have all been accepted under the new active key.

Windows remains denied with `windows_physical_gate_required` even when the code path is otherwise
valid. That reason can only be removed after the physical Windows/WSL2 multi-GPU isolation gate is
implemented and passed; this slice does not add a configuration bypass.

## Scheduler consumption

`POST /v1/scheduler/run` evaluates Runtime Admission for the job's exact provider, device and GPU
inside the transaction that may create its lease. The scheduler uses one server `now` for the
entire pass. A denied decision produces `decision=skipped`, no `lease_id` and the admission reason
codes; only `status=admitted` can proceed to GPU locking and lease insertion.

The request `limit` is the maximum number of leases offered, not a pre-admission row cutoff.
Candidates are fetched in batches of 50 up to a bounded evaluation budget of 50-800 per pass.
Migration `0028_scheduler_runtime_admission` adds `scheduler_last_evaluated_at`; evaluated jobs
rotate behind untouched candidates so a denied prefix cannot starve later admitted jobs across
runs. Transaction-scoped advisory GPU locks plus the active-lease unique indexes keep concurrent
scheduler passes fail-closed.

Offered-lease audit metadata records the non-secret Runtime Admission `verification_id`,
verification fingerprint, observation hash and evaluation time. The scheduler does not call the
admin listing endpoint or cache admission as independent authority.

## Assignment revalidation

`GET /v1/sessions/{session_id}/jobs/next` reuses the same transaction-aware evaluator immediately
before job credential issuance. A fresh decision may use a newer valid proof than the scheduler
used. The endpoint locks at most 16 offered leases and queued jobs per poll and continues past
denied offers, preventing one stale GPU from blocking another admitted GPU for the session.

If current admission is denied, no credential or bundle is created. The lease becomes `expired`
with `runtime_admission_lost_before_assignment`, the job remains `queued`, credential fields are
cleared, and `lease.assignment_withheld` records the current non-secret decision. Only an admitted
decision can reach the linearization point that hashes the new credential and moves the locked job
to `assigned` in the same commit.

## Versioning and migration

Migration `0027_runtime_verified_admission` adds immutable runtime observations and extends runtime
verification records with `public_key_id`, `runtime_admission_fingerprint` and canonical admission
claims. Active v1 records lacking those bindings are superseded fail-closed and must be reproved.

The approved proof image is now Control Plane policy. Challenge issuance fails when the image is
unconfigured or the administrator request differs from the configured digest.

## Deferred work

- production Provider Job Worker activation;
- physical Linux and Windows gates and Windows admission enablement;
- hardware-backed Agent integrity/remote attestation;
- marketplace, billing or autonomous challenge scheduling changes.
