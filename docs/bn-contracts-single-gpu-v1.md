# BN-CONTRACTS - Single-GPU v1

This document freezes the contract gaps that must be closed before Burd exposes a
customer-facing single-GPU compute API. It is an audit/design contract only: it
does not add routes, migrations, behavior, pricing, billing execution, Pix
settlement, provider payouts, scheduler matching, or production readiness.

## Scope and Non-goals

Scope:

- single customer workload -> one backend placement -> one physical
  `ComputeJob` -> one `JobAttempt` -> one provider `Lease` -> one GPU UUID;
- customer-facing compute contracts that hide physical provider/device/session/GPU
  identifiers from customer requests;
- current contract inventory for provider identity, marketplace listings,
  reservations, jobs, leases, usage, billing primitives, OpenAPI, and errors;
- precise boundaries between implemented backend authority and proposed public
  API contracts.

Non-goals:

- no Pix gateway execution, banking integration, automatic settlement, or payout
  delivery;
- no multi-GPU, multi-provider, distributed inference, or distributed training;
- no new product feature, endpoint, migration, schema, API behavior, or UI;
- no claim that the current system is production-ready;
- no relaxation of NVIDIA physical gates or worker activation prerequisites.

## Audit Corrections

- Current `/v1/jobs` is not a customer compute API. `CreateJobRequest` in
  `crates/burd-protocol/src/job.rs` requires `provider_id`, `device_id`,
  `session_id`, and `gpu_uuid`; migration
  `crates/burd-control-plane/migrations/0012_job_api_data_plane.sql` stores the
  same physical fields on `compute_jobs`.
- Current marketplace reservations are physical-bound holds. `CreateReservationRequest`
  accepts `listing_id`, `duration_seconds`, optional `starts_at`, and optional
  `workload_type`, while `MarketplaceReservationRecord` exposes backend-derived
  `provider_id`, `device_id`, optional `session_id`, and optional `gpu_uuid` in
  `crates/burd-protocol/src/customer.rs`.
- Current scheduler is a lease arbiter over already-directed queued jobs.
  `crates/burd-control-plane/src/scheduler.rs` selects queued `compute_jobs`,
  checks backend workload eligibility and runtime admission, and inserts
  `job_leases`; it does not choose abstract marketplace supply for a customer
  workload.
- Current assignment and acceptance authority is exact-lease-bound.
  `AcceptJobRequest` requires `lease_id`, and migration
  `crates/burd-control-plane/migrations/0030_compute_job_assignment_lease_binding.sql`
  binds active `compute_jobs.assignment_lease_id` to `job_leases`.
- Current billing/payment/payout chain is PARTIAL, not missing. Billing
  primitives, immutable pricing snapshots, Pix payment intents, financial ledger
  lines, invoices, payout accounts, and payout records exist, but real external
  money movement is not the public production contract.
- Current OpenAPI is PARTIAL, not missing. `crates/burd-control-plane/src/openapi.rs`
  manually defines `OpenAPI 3.1.0`, security schemes, error envelope, idempotency
  parameter, and many route schemas; serde/OpenAPI parity is not yet generated
  or golden-fixture-enforced.
- Current versioning is PARTIAL, not missing. Protocol structs carry many
  `schema_version` constants, but there is no public capability-negotiation
  protocol for customers, agents, and control-plane contract versions.

## Current Implementation Matrix

| Domain | Contract | State | Evidence | Gap | Target PR |
| --- | --- | --- | --- | --- | --- |
| Provider identity | Provider, device, enrollment, key rotation, revocation | IMPLEMENTED | `crates/burd-control-plane/src/http.rs`, `crates/burd-control-plane/src/enrollment.rs` | Public customer compute must not reuse provider enrollment contracts | BN-CONTRACTS-01 |
| Remote session | Session start, heartbeat, control channel, revoke | IMPLEMENTED | `crates/burd-control-plane/src/http.rs`, `crates/burd-protocol/src/remote_session.rs` | Customer workload status projection should not expose session internals | BN-CONTRACTS-03 |
| GPU inventory | Signed snapshots, including zero-GPU snapshot | IMPLEMENTED | `crates/burd-control-plane/migrations/0029_gpu_inventory_authoritative_snapshots.sql` | Customer API needs abstract supply summaries only | BN-CONTRACTS-02 |
| Runtime admission | Backend admission from proof, observation, inventory | IMPLEMENTED | `crates/burd-control-plane/src/runtime_admission.rs` | Placement needs to consume admission without exposing raw proof fields | BN-CONTRACTS-02 |
| Marketplace listing | Backend-derived listing registry | IMPLEMENTED | `crates/burd-protocol/src/marketplace.rs`, `crates/burd-control-plane/migrations/0015_marketplace_registry_listings.sql` | Listing is supply inventory, not customer workload submission | BN-CONTRACTS-02 |
| Reservation | Physical marketplace hold against listing | PARTIAL | `crates/burd-protocol/src/customer.rs`, `crates/burd-control-plane/migrations/0016_customer_accounts_reservations.sql` | Reservation must bind to workload and pricing snapshot, not raw listing alone | BN-CONTRACTS-03 |
| Customer workload | Public workload intent | MISSING | No `Workload` or `ComputeRequest` contract in `crates/burd-protocol/src` | Need customer-owned workload resource | BN-CONTRACTS-01 |
| Compute requirements | Public GPU/runtime/perf constraints | MISSING | Current job request requires physical IDs in `crates/burd-protocol/src/job.rs` | Need abstract requirements, no physical IDs from customer | BN-CONTRACTS-01 |
| Placement | Backend choice of supply for workload | PROPOSED | Scheduler currently offers leases for directed jobs in `crates/burd-control-plane/src/scheduler.rs` | Need backend-owned placement resource before job creation | BN-CONTRACTS-02 |
| Compute job | Physical execution record | IMPLEMENTED | `compute_jobs` in migration `0012_job_api_data_plane.sql` | Keep internal/admin-directed; project customer status from it | BN-CONTRACTS-03 |
| Job attempt | Retry/attempt boundary | MISSING | `compute_jobs` has one job lifecycle; no `job_attempts` table | Need attempt resource to separate retries from logical workload | BN-CONTRACTS-03 |
| Lease | Scheduler-offered physical authority | IMPLEMENTED | `crates/burd-protocol/src/lease.rs`, `crates/burd-control-plane/src/scheduler.rs` | Lease remains backend/provider internal, not customer-submitted | BN-CONTRACTS-02 |
| Job execution control | Active-job continue/cancel directive | IMPLEMENTED | `crates/burd-protocol/src/job_execution.rs` | Customer sees projected cancellation, not device control details | BN-CONTRACTS-03 |
| Usage metering | Append-only usage receipt ledger | IMPLEMENTED | `crates/burd-control-plane/migrations/0014_usage_metering_ledger.sql` | Need workload/reservation/job-attempt linkage for customer invoices | BN-CONTRACTS-04 |
| Customer accounts | Organizations, users, projects, keys, quotas | IMPLEMENTED | `crates/burd-protocol/src/customer.rs`, migration `0016_customer_accounts_reservations.sql` | Human auth/RBAC is coarse and admin/bootstrap-heavy | BN-CONTRACTS-05 |
| Customer credits | Non-settlement credit ledger | IMPLEMENTED | `customer_credit_ledger_entries` in migration `0016_customer_accounts_reservations.sql` | Credit semantics need financial/accounting mapping for public compute | BN-CONTRACTS-04 |
| Financial ledger | Double-entry ledger lines | IMPLEMENTED | `financial_ledger_lines` in migration `0017_billing_pix_payouts.sql` | Needs pricing snapshot and customer workload source ids | BN-CONTRACTS-04 |
| Pricing | Active listing price book | PARTIAL | `marketplace_listing_prices` in migration `0017_billing_pix_payouts.sql` | Invoice uses active price; immutable snapshot is proposed | BN-CONTRACTS-04 |
| Pix payment intent | Stored provider/external reference intent | PARTIAL | `crates/burd-protocol/src/billing.rs`, `crates/burd-control-plane/src/billing.rs` | External adapter and settlement workflow remain outside core ledger | BN-CONTRACTS-06 |
| Provider payout | Account and payout records | PARTIAL | `provider_payout_accounts`, `provider_payouts` in migration `0017_billing_pix_payouts.sql` | No banking execution or paid/failed/cancelled adapter transition API | BN-CONTRACTS-06 |
| OpenAPI | Manual API document | PARTIAL | `crates/burd-control-plane/src/openapi.rs` | Needs serde/OpenAPI parity and golden fixture gates | BN-CONTRACTS-07 |
| Version negotiation | Schema constants on payloads | PARTIAL | `*_SCHEMA_VERSION` constants in `crates/burd-protocol/src` | Need `/v1` capability negotiation and deprecation policy | BN-CONTRACTS-07 |

Implementation update: BN-CUSTOMER-COMPUTE-01 now implements the first
customer workload, abstract single-GPU requirements, transactional placement,
and workload/placement-to-directed-job bridge in
`crates/burd-protocol/src/customer_compute.rs`,
`crates/burd-control-plane/src/customer_compute.rs`, and migration `0031`.
Reservation binding, customer artifacts/status, immutable pricing, and job
attempts remain follow-up work.

## Resource Model

### Identity and Human

- Current: `users`, `organizations`, `organization_users`, `projects`,
  `customer_api_keys`.
- Authority: backend stores human/customer records; current admin bearer can
  create users, organizations, projects, quotas, and API keys.
- Target: explicit human auth sessions and RBAC roles for organization owner,
  project admin, project developer, billing admin, support admin, provider admin,
  and read-only auditor.

### Provider

- Current: provider, device, public keys, remote sessions, telemetry, evidence,
  GPU inventory, runtime observations, runtime verifications, workload
  eligibility, trust, antifraud, benchmark results, and leases.
- Authority: provider/agent can claim local facts only through signed payloads;
  backend derives admission, trust, listing, lease, and acceptance authority.
- Target: provider resources remain internal supply. Customers never submit
  `provider_id`, `device_id`, `session_id`, or `gpu_uuid` to start public compute.

### Customer

- Current: organization/project/API-key/quota/credit/reservation primitives.
- Target: customer owns `Workload`, artifacts, desired requirements, billing
  project, and cancellation intent. Customer receives projected state/events, not
  physical execution internals.

### Marketplace

- Current: backend-derived `marketplace_listings` and active price book.
- Target: listing is supply discovery input. Public compute starts from
  `Workload` and `ComputeRequirements`; backend placement chooses supply.

### Compute

- Current: `compute_jobs` is a physical execution record; `job_leases` is the
  physical execution authority; assignment and acceptance are exact-lease-bound.
- Target: public chain is `Reservation -> Workload -> Placement -> ComputeJob ->
  JobAttempt -> Lease`, with `ComputeJob` internal and customer-facing status
  projected from attempts, events, artifacts, and terminal usage.

### Billing and Payments

- Current: non-settlement customer credits, immutable pricing snapshots,
  financial ledger lines, Pix payment intents, invoices, payout accounts,
  payouts, refund/dispute/reconciliation placeholders.
- Current `PricingSnapshot` binds reservation/workload/job/usage/invoice to
  quoted price, currency, pricing model, fees, reserve policy, source listing,
  source price, and policy version. Legacy reservations without snapshots fail
  closed at settlement.

### API and OpenAPI

- Current: Axum routes in `crates/burd-control-plane/src/http.rs`, serde
  contracts in `crates/burd-protocol/src`, manual OpenAPI in
  `crates/burd-control-plane/src/openapi.rs`.
- Target: route, serde, JSON examples, OpenAPI schemas, idempotency docs, auth
  scopes, and error examples must be checked together by golden fixtures.

## Authority Matrix

| Resource/field | Classification | Current source | Public rule |
| --- | --- | --- | --- |
| Customer workload prompt/parameters | customer-claimed | PROPOSED `Workload` | Validated, stored, and redacted by backend policy |
| Customer artifact manifest | customer-claimed then backend-verified | `customer_artifacts` plus private object store | Backend verifies exact size/SHA-256 and owns the provider manifest; content scanning and external grants remain future work |
| Customer organization/project ids | backend-authoritative | `organizations`, `projects` | Customer token scopes constrain access |
| Customer API key token | never public after creation | `CreateCustomerApiKeyResponse.token` | Return once; store only hash/prefix |
| Provider registration payload | provider-claimed | enrollment payload | Not trusted until proof of possession and backend enrollment |
| Device public key and signatures | agent-signed | `provider_public_keys`, signed payloads | Verify signature, key status, nonce/session binding |
| GPU UUID/inventory | agent-signed then backend-attested | signed GPU inventory snapshots | Customer sees abstract verified supply, not raw assignment input |
| Runtime admission | backend-derived | `runtime_admission` evaluation | Scheduler/placement consumes this authority |
| Marketplace listing status | backend-derived | `marketplace_listings` sweep | Customer may filter listings, not force publication |
| Reservation status | backend-authoritative | `marketplace_reservations` | Customer can request/cancel within scope; backend owns terminal state |
| Placement decision | backend-authoritative | PROPOSED `Placement` | Customer supplies requirements; backend chooses provider/device/GPU |
| `provider_id`/`device_id`/`session_id`/`gpu_uuid` for execution | backend-authoritative | current `compute_jobs` and `job_leases` | Never accepted from future public customer compute API |
| Job credential | never public to customer | `JobDataPlaneGrant.credential` | Provider-side only; hash-only at rest |
| Job events | provider-claimed then backend-attested | `job_events` plus customer workload event projection | Customer sees bounded, sanitized events without physical bindings or internal metadata |
| Usage receipt | backend-derived/backend-attested | `usage_ledger_entries` | Billing consumes append-only usage |
| Active listing price | backend-authoritative | `marketplace_listing_prices` | Must be snapshotted before invoice in target contract |
| Invoice totals | backend-derived | `billing_invoices` | Customer/provider cannot submit totals |
| Pix intent external reference | customer-claimed input, backend-validated | optional `CreatePixPaymentIntentRequest.external_reference` stored on `pix_payment_intents` | Customer may provide a bounded reference, but it does not move ledger funds |
| Pix confirmation external reference | backend-authoritative after admin/adapter input | `ConfirmPixPaymentIntentRequest.external_reference` updates a payment intent to `confirmed` after validation | Admin/payment-adapter input is not a compute-provider claim; ledger changes only through backend policy |
| Payout external reference | backend-authoritative after admin/adapter reconciliation | `provider_payouts.external_reference` and reconciliation placeholders | External payout references are adapter/admin inputs validated by backend; compute providers cannot mark payout paid |
| Error request_id | backend-authoritative | `ApiError` | Always included in error envelope |

## Customer Workload, ComputeRequirements, Placement, and ComputeJob Boundaries

Current `CreateJobRequest` is physical and must remain an admin/internal contract:

- customer must not send `provider_id`, `device_id`, `session_id`, or `gpu_uuid`;
- customer must not choose lease, runtime admission evidence, job credential, or
  control-channel identity;
- customer may provide workload intent, accepted template/workload type, artifact
  references, runtime class, region preference, budget/price constraints, timeout,
  and policy constraints after validation.

Target boundaries:

- `Workload`: customer-owned logical request, artifacts, workload type,
  parameters, timeout, cancellation intent, and desired outputs.
- `ComputeRequirements`: customer-visible constraints such as GPU class, minimum
  VRAM, backend family, region, latency/network class, trust tier, budget, and
  capability profile. It is not a physical assignment.
- `Placement`: backend-owned decision linking a workload/reservation to one
  verified supply candidate. Placement consumes listings, admission, trust,
  price snapshot, quotas, and policy.
- `ComputeJob`: backend/internal physical execution record derived from
  placement. It can contain provider/device/session/GPU/lease/job credential.
- `JobAttempt`: proposed retry boundary under a workload/job. Attempts carry
  execution attempts, provider assignment changes, retries, and failure
  classification without changing the customer workload identity.

## Reservation -> Workload -> Placement -> Job -> JobAttempt -> Lease Invariants

- A `Reservation` reserves commercial capacity for one project and must bind to a
  `PricingSnapshot` before billable execution.
- A `Workload` is customer-owned and can exist before placement.
- A `Placement` is backend-owned and chooses exactly one physical provider/device
  GPU for single-GPU v1.
- A `ComputeJob` is created only from an admitted placement; public customers do
  not create physical jobs directly.
- A `JobAttempt` is created for each execution try; retries create new attempts,
  not a new customer workload.
- A `Lease` is created by scheduler/placement authority and binds one attempt to
  one provider/device/session/GPU for a bounded TTL.
- Active assignment requires exact `assignment_lease_id`; stale acknowledgements
  must not mutate a newer assignment.
- Terminal workload state must be projected from terminal attempt/job state plus
  artifact and usage outcomes.

## Customer Artifact Ingress

Current job artifacts are internal job artifact manifests:

- `JobArtifact` includes `artifact_id`, `role`, `object_key`, optional `sha256`,
  optional `size_bytes`, and optional `content_type`;
- data-plane grants and upload/download URLs are provider/job credential bound.

Implemented customer artifact ingress:

- customer creates a project-scoped upload intent and uploads through the
  customer-authenticated Control Plane; the initial adapter is the existing
  private filesystem object store;
- backend stores artifact metadata before placement;
- exact size and SHA-256 are verified on upload and rechecked from storage during
  idempotent finalize before status becomes `ready`;
- workload input references are restricted to ready, unexpired artifacts owned
  by the same project and are converted to backend-owned internal job manifests;
- backend never exposes object keys or provider data-plane credentials to the
  customer. Provider access remains job-credential scoped;
- current public statuses are `pending_upload`, `uploaded`, `ready`, `expired`,
  and `rejected`. Content scanning, explicit deletion, output download, and
  external object-storage grants remain future work.

## Customer Job Status and Events Projection

Implemented customer routes expose one project-owned workload/job projection,
bounded sanitized events, and idempotent cancellation. Internal assignment
states are collapsed, and provider/device/session/GPU/lease credentials,
fingerprints, object keys, policies, image refs, and raw event metadata are not
part of the customer contract. Result artifact metadata is public, but customer
output download remains future work.

Current provider job events are physical `job_events` with provider/device/session
fields and sequence numbers. Target public projection:

- customer sees workload/job status, progress, sanitized event type, timestamp,
  artifact state, cancellation state, and terminal reason category;
- customer does not see provider session token, job credential, raw device
  credential, raw runtime observation, raw proof evidence, or private provider
  telemetry;
- provider event streams are backend-attested before becoming customer-visible;
- customer cancellation request maps to backend cancellation authority and active
  job control, not direct provider command.

## Human Auth and RBAC

Current auth is PARTIAL:

- admin bearer protects admin routes in `crates/burd-control-plane/src/http.rs`;
- customer API keys are project-scoped and scope-checked for current customer and
  billing routes;
- device/session bearer credentials protect provider session routes.

Target RBAC:

- human sessions and API keys must have separate scopes and audit trails;
- roles: organization owner, organization admin, project admin, project developer,
  billing admin, support admin, provider admin, read-only auditor;
- customer keys cannot call provider/device/session/job-control admin routes;
- provider session credentials cannot call customer, billing admin, or marketplace
  administration routes;
- admin support actions require actor id and audit event.

## Immutable Pricing Snapshot

Billing settlement now uses immutable `pricing_snapshots` instead of loading the
active listing price at settlement time. Current `PricingSnapshot`:

- created when reservation or workload quote is accepted;
- immutable after creation;
- includes `pricing_snapshot_id`, `listing_id`, `source_price_id`, currency,
  price model, price_per_hour_micros, platform fee bps, reserve bps, fee policy
  version, and created_at;
- `billing_invoices` references the snapshot rather than re-reading the active
  price book;
- repeated settlement must verify usage, reservation, placement/job, and pricing
  snapshot binding before appending financial ledger lines.

## Version and Capability Negotiation

Current versioning is PARTIAL:

- protocol payloads carry constants such as `burd-job-v1`,
  `burd-job-lease-v1`, `burd-provider-job-execution-v3`, and billing/customer
  schema versions;
- OpenAPI declares API version `v1`.

Target negotiation:

- `/v1/capabilities` or equivalent returns supported protocol families,
  minimum/maximum schema versions, deprecation windows, and required feature
  flags;
- Agent handshake advertises execution, inventory, telemetry, runtime proof,
  cancellation, artifact, and platform capabilities;
- customer clients advertise API contract version and optional preview features;
- incompatible versions fail closed with `409 conflict` or `422 invalid_request`
  according to the concrete failure.

## Pix and Payout External Boundary

Current Pix/payment/payout is PARTIAL:

- `pix_payment_intents` stores provider/external reference and confirmation data;
- `financial_ledger_lines` is append-only;
- `provider_payout_accounts` and `provider_payouts` store payout account and
  payout records;
- refund/dispute/reconciliation tables are bounded placeholders.

Boundary rule:

- the financial ledger is Burd-owned; Pix providers and banks are adapters;
- external payment confirmation can trigger backend policy, but cannot directly
  write ledger lines;
- payout execution status must come through an adapter/admin reconciliation API
  that validates provider, external reference, amount, currency, idempotency, and
  prior payout state;
- provider cannot mark itself paid, change invoice totals, or bypass holds.

## Endpoint Idempotency Matrix

Retention is not globally frozen. Current idempotency responses are persisted in
`idempotency_keys` for endpoints that explicitly reserve a scoped key; retention
duration and pruning policy must be frozen before public API launch.

| Endpoint | State | Auth/scope | Idempotency | Key scope/format | Same payload | Different payload | Retention/stored response |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `POST /v1/providers` | IMPLEMENTED | admin bearer | required | scope per provider create, `Idempotency-Key`, max 128 printable ASCII | replays stored response | `409 idempotency_conflict` | stored response; retention not frozen |
| `POST /v1/providers/{provider_id}/enrollment-tokens` | IMPLEMENTED | admin bearer | not applicable | N/A | creates new short token | N/A | no idempotent replay |
| `POST /v1/enrollments` | IMPLEMENTED | enrollment token bearer | not applicable | N/A | creates enrollment attempt | N/A | no idempotent replay |
| `POST /v1/enrollments/{enrollment_id}/proof` | IMPLEMENTED | enrollment flow credential | not applicable | N/A | state-dependent proof completion | conflict/invalid on mismatch | no idempotent replay |
| `POST /v1/devices/{device_id}/credentials` | IMPLEMENTED | device bearer | not applicable | N/A | refreshes credential | N/A | no idempotent replay |
| `POST /v1/devices/{device_id}/key-rotations` | IMPLEMENTED | device bearer | not applicable | N/A | creates rotation | N/A | no idempotent replay |
| `POST /v1/devices/{device_id}/key-rotations/{rotation_id}/proof` | IMPLEMENTED | device bearer | not applicable | N/A | state-dependent completion | conflict/invalid on mismatch | no idempotent replay |
| `POST /v1/devices/{device_id}/revoke` | IMPLEMENTED | admin bearer | not applicable | N/A | revokes or reports terminal state | N/A | no idempotent replay |
| `POST /v1/sessions` | IMPLEMENTED | device credential | not applicable | N/A | creates remote session | duplicate/session policy applies | no idempotent replay |
| `POST /v1/sessions/{session_id}/heartbeats` | IMPLEMENTED | session headers | not applicable | N/A | sequence/state update | conflict/invalid sequence | no idempotent replay |
| `POST /v1/sessions/{session_id}/revoke` | IMPLEMENTED | admin bearer | not applicable | N/A | revokes session | N/A | no idempotent replay |
| `POST /v1/sessions/{session_id}/telemetry-batches` | IMPLEMENTED | session headers | not applicable | N/A | batch sequence validation | conflict/invalid sequence | no idempotent replay |
| `POST /v1/sessions/{session_id}/security-posture` | IMPLEMENTED | session headers | optional by payload hash behavior, not header | signed hash | duplicate signed envelope replays duplicate response | binding/hash conflict rejected | stored in domain tables, not idempotency_keys |
| `POST /v1/sessions/{session_id}/gpu-inventory` | IMPLEMENTED | session headers | optional by inventory hash behavior, not header | signed `inventory_hash` | duplicate returns duplicate response | binding/hash conflict rejected | stored in domain tables, not idempotency_keys |
| `POST /v1/sessions/{session_id}/evidence-records` | IMPLEMENTED | session headers | optional by evidence hash behavior, not header | signed report hash | duplicate returns duplicate response | hash/binding conflict rejected | stored in domain tables, not idempotency_keys |
| `POST /v1/evidence-records/{evidence_id}/revoke` | IMPLEMENTED | admin bearer | not applicable | N/A | revokes evidence | N/A | no idempotent replay |
| `POST /v1/network-probes/observations` | IMPLEMENTED | admin bearer | no `Idempotency-Key`; no signature/hash in request | database uniqueness on `(session_id, probe_id, observed_at)` | `ON CONFLICT DO NOTHING` returns the existing observation and `duplicate=true` | conflicting payload with the same `(session_id, probe_id, observed_at)` is not compared, so the first stored payload wins silently | domain storage; weak duplicate semantics to harden before public probe authority |
| `POST /v1/benchmark-profiles` | IMPLEMENTED | admin bearer | not applicable | N/A | upsert profile | overwrites/upserts according to key | no idempotent replay |
| `POST /v1/workload-policies` | IMPLEMENTED | admin bearer | not applicable | N/A | upsert policy | overwrites/upserts according to key | no idempotent replay |
| `POST /v1/sessions/{session_id}/benchmark-results` | IMPLEMENTED | session headers | optional by result hash behavior | signed result hash | duplicate response | binding/hash conflict rejected | domain storage |
| `POST /v1/workload-eligibility/sweep` | IMPLEMENTED | admin bearer | not applicable | N/A | recomputes state | N/A | no idempotent replay |
| `POST /v1/marketplace/listings/sweep` | IMPLEMENTED | admin bearer | not applicable | N/A | recomputes listings | N/A | no idempotent replay |
| `POST /v1/marketplace/listings/{listing_id}/price` | IMPLEMENTED | admin bearer | not applicable | N/A | upserts active price | replaces active listing price | no idempotent replay |
| `POST /v1/customer/users` | IMPLEMENTED | admin bearer | not applicable | N/A | creates user | N/A | no idempotent replay |
| `POST /v1/customer/organizations` | IMPLEMENTED | admin bearer | not applicable | N/A | creates organization | N/A | no idempotent replay |
| `POST /v1/customer/organizations/{organization_id}/projects` | IMPLEMENTED | admin bearer | not applicable | N/A | creates project | N/A | no idempotent replay |
| `POST /v1/customer/projects/{project_id}/quotas` | IMPLEMENTED | admin bearer | not applicable | N/A | upserts quota | replaces quota | no idempotent replay |
| `POST /v1/customer/projects/{project_id}/api-keys` | IMPLEMENTED | admin bearer | not applicable | N/A | creates token returned once | N/A | no idempotent replay |
| `POST /v1/customer/projects/{project_id}/credits` | IMPLEMENTED | admin bearer | required | scope per project credit grant and key | replays stored ledger response | `409 idempotency_conflict` | stored response; retention not frozen |
| `POST /v1/customer/projects/{project_id}/reservations` | IMPLEMENTED | customer bearer `reservations:write` | required | scope per project reservation and key | replays stored reservation response | `409 idempotency_conflict` | stored response; retention not frozen |
| `POST /v1/customer/reservations/{reservation_id}/cancel` | IMPLEMENTED | customer bearer `reservations:write` | not applicable | N/A | cancels when current status is `reserved`; if the reservation is already not `reserved`, returns current reservation with `duplicate=true` | no payload conflict detection for repeated cancel reason; not a replay from stored response | no `Idempotency-Key`; state-based duplicate response |
| `POST /v1/billing/projects/{project_id}/pix/payment-intents` | IMPLEMENTED | customer bearer `billing:write` | required | scope per project payment intent and key | replays stored payment intent | `409 idempotency_conflict` | stored response; retention not frozen |
| `POST /v1/billing/pix/payment-intents/{payment_intent_id}/confirm` | IMPLEMENTED | admin bearer | not header-keyed | external provider/reference | same confirmation is idempotent by stored confirmation | conflicting confirmation returns `409 conflict` | domain state |
| `POST /v1/billing/reservations/{reservation_id}/settle` | IMPLEMENTED | admin bearer | not header-keyed | unique reservation/usage pair | returns existing invoice for same usage | `409 conflict` for conflicting usage/reservation | domain uniqueness |
| `POST /v1/billing/providers/{provider_id}/payout-account` | IMPLEMENTED | admin bearer | not applicable | N/A | upserts account | replaces account fields according to policy | no idempotent replay |
| `POST /v1/billing/providers/{provider_id}/payouts` | IMPLEMENTED | admin bearer | not header-keyed | payout record | creates payout if policy permits | conflict if policy/balance insufficient | domain state |
| `POST /v1/jobs` | IMPLEMENTED | admin bearer | required | scope per provider/client job/idempotency key | replays stored job response | `409 idempotency_conflict` | stored response; retention not frozen |
| `POST /v1/jobs/{job_id}/cancel` | IMPLEMENTED | admin bearer | not applicable | N/A | terminalizes/cancels by state | conflict if not cancellable | no idempotent replay |
| `PUT /v1/jobs/{job_id}/results/{artifact_id}/upload` | IMPLEMENTED | job data-plane bearer | not applicable | N/A | writes result artifact | authorization/path/hash validation | no idempotent replay |
| `POST /v1/jobs/{job_id}/usage-ledger/finalize` | IMPLEMENTED | admin bearer | not header-keyed | unique `(job_id, entry_type)` | returns existing usage ledger entry | conflict on invalid terminal state | domain uniqueness |
| `POST /v1/scheduler/run` | IMPLEMENTED | admin bearer | not applicable | N/A | offers leases for queued jobs | N/A | no idempotent replay |
| `GET /v1/sessions/{session_id}/jobs/next` | IMPLEMENTED | device/session auth | no `Idempotency-Key` | exact session headers | may lock up to the assignment window, revalidate runtime admission, assign a queued job, persist `assignment_lease_id`, hash/store a new `jobcred`, and audit `job.assigned`; if no valid offer remains, returns empty assignment | repeated polling can return no job after assignment or continue to search after withheld authority; not a replay endpoint | side effects are real despite GET; no stored idempotency response |
| `POST /v1/sessions/{session_id}/jobs/{job_id}/accept` | IMPLEMENTED | session headers | not applicable | exact `lease_id` in body | first valid accept transitions state | stale/current-lost authority returns conflict | no idempotent replay |
| `GET /v1/sessions/{session_id}/jobs/{job_id}/control` | IMPLEMENTED | device/session auth | no `Idempotency-Key` | exact `job_id` path, `lease_id` query parameter, and session headers | reads execution directive `continue` or `cancel` for the exact assignment | stale authority, wrong lease, replaced assignment, or inactive state fails closed with conflict instead of inferring latest lease | no stored idempotency response |
| `POST /v1/sessions/{session_id}/jobs/{job_id}/events` | IMPLEMENTED | session headers | sequence-keyed | unique `(job_id, sequence)` | duplicate sequence rejected/conflict | different event with same sequence rejected | domain uniqueness |
| `POST /v1/sessions/{session_id}/jobs/{job_id}/result` | IMPLEMENTED | session headers | not applicable | N/A | terminal result by state | conflict if not valid | no idempotent replay |
| `POST /v1/trust/sweep` | IMPLEMENTED | admin bearer | not applicable | N/A | recomputes state | N/A | no idempotent replay |
| `POST /v1/verification/sweep` | IMPLEMENTED | admin bearer | not applicable | N/A | recomputes state/issues challenges | N/A | no idempotent replay |
| `POST /v1/challenges` | IMPLEMENTED | admin bearer | not applicable | N/A | issues challenge | N/A | no idempotent replay |
| `POST /v1/sessions/{session_id}/challenges/{challenge_id}/response` | IMPLEMENTED | session headers | challenge-bound | one response per challenge | repeated same terminal state rejected or reported by domain policy | conflicting response rejected | domain state |
| `POST /v1/runtime-verifications/challenges` | IMPLEMENTED | admin bearer | not applicable | N/A | issues runtime challenge | N/A | no idempotent replay |
| `POST /v1/sessions/{session_id}/runtime-observations` | IMPLEMENTED | session headers | optional by observation hash behavior | signed observation hash | duplicate response | binding/hash conflict rejected | domain storage |
| `POST /v1/sessions/{session_id}/runtime-verifications/{challenge_id}/response` | IMPLEMENTED | session headers | challenge-bound | one response per challenge | repeated terminal state rejected or reported by domain policy | conflicting response rejected | domain state |
| `POST /v1/customer/projects/{project_id}/workloads` | IMPLEMENTED | customer bearer `workloads:write` | required | project + idempotency key | replays stored workload/placement/job response | `409 idempotency_conflict` | stored response; retention not frozen |
| `GET /v1/customer/projects/{project_id}/workloads/{workload_id}` | IMPLEMENTED | customer bearer `workloads:read` | not applicable | exact organization/project/workload ownership | returns the current sanitized workload/job projection | N/A | no provider/device/session/GPU, credential, object key, fingerprint, raw status message, or internal metadata |
| `GET /v1/customer/projects/{project_id}/workloads/{workload_id}/events` | IMPLEMENTED | customer bearer `workloads:read` | not applicable | exact organization/project/workload ownership | returns bounded sanitized event projections ordered by sequence | N/A | raw messages and event metadata are omitted |
| `POST /v1/customer/projects/{project_id}/workloads/{workload_id}/cancel` | IMPLEMENTED | customer bearer `workloads:write` | no `Idempotency-Key` | exact organization/project/workload ownership and locked job state | an already-cancelled job returns the current projection with `duplicate=true` | terminal `succeeded/failed` jobs return conflict; repeated reasons are not compared | state-based idempotency; clears job credential, terminalizes lease, releases placement/reservation, finalizes usage, and leaves history intact |
| `POST /v1/customer/projects/{project_id}/artifacts` | IMPLEMENTED | customer bearer `artifacts:write` | required | project + idempotency key | replays stored upload intent | `409 idempotency_conflict` | stored response; retention not frozen |
| `PUT /v1/customer/projects/{project_id}/artifacts/{artifact_id}/content` | IMPLEMENTED | customer bearer `artifacts:write` | not applicable | exact project-owned pending artifact | repeated complete upload is rejected by lifecycle | size/lifecycle conflict | private storage path is backend-owned |
| `POST /v1/customer/projects/{project_id}/artifacts/{artifact_id}/finalize` | IMPLEMENTED | customer bearer `artifacts:write` | not applicable | exact project-owned uploaded artifact | ready artifact returns `duplicate=true` | stored size/hash mismatch fails closed | finalize recomputes bytes from private storage |

## Public State Machines

| Resource | State | Actor | Preconditions | Side effects | Terminal |
| --- | --- | --- | --- | --- | --- |
| Reservation | IMPLEMENTED current: `reserved -> cancelled/expired`; PROPOSED: `quoted -> reserved -> bound_to_workload -> consumed -> released/cancelled/expired` | Customer/admin/backend | Active project, quota, listing, pricing snapshot in target | Listing hold, credit/ledger marker, audit | `consumed`, `released`, `cancelled`, `expired` |
| Workload | IMPLEMENTED: `queued -> placed -> succeeded/failed/cancelled`; placement failure is `placement_failed` before a job exists | Customer/backend | Valid project, API key scope, requirements, ready project-owned artifacts, optional compatible reservation | Creates a backend-owned placement and ComputeJob; terminal job state is projected back to the workload | `placement_failed`, `succeeded`, `failed`, `cancelled` |
| Placement | IMPLEMENTED: `selected -> released`; failed selection creates no placement | Backend | Workload request, optional reservation, compatible listing and current Runtime Admission | Chooses listing/provider/device/session/GPU internally and prevents concurrent selected placement per GPU | `released` |
| ComputeJob | IMPLEMENTED: `queued -> assigned -> accepted -> provisioning -> running -> uploading -> succeeded/failed/cancelled`; authority loss can requeue/withhold and clear credentials, while lease expiry is tracked on `Lease`; PROPOSED customer projection may expose `expired` | Admin/backend/provider session | Physical provider/device/session/GPU, runtime admission, lease authority | Job credential, events, artifacts, usage | current job terminals: `succeeded`, `failed`, `cancelled`; projected `expired` is PROPOSED |
| JobAttempt | PROPOSED: `pending -> offered -> assigned -> accepted -> provisioning -> running -> uploading -> succeeded/failed/cancelled/expired` | Backend/provider session | Placement selected, retry budget available | One lease and one physical execution try | `succeeded`, `failed`, `cancelled`, `expired` |
| Lease | IMPLEMENTED: `offered -> accepted -> provisioning -> active -> completed/expired/failed` | Scheduler/provider session/backend | Queued physical job, admission, no active GPU lease | Binds execution authority and `assignment_lease_id` | `completed`, `expired`, `failed` |
| Artifact | IMPLEMENTED customer input: `pending_upload -> uploaded -> ready`; `expired/rejected` reserved terminal states. Provider result artifacts remain internal. | Customer/backend/provider | Project ownership, exact size/hash, upload and retention expiry | Private object-store write, finalize recheck, workload binding | `expired`, `rejected` |
| Payment | PARTIAL: `requires_confirmation -> confirmed`; PROPOSED: `requires_confirmation -> pending_provider -> confirmed -> expired/failed/refunded` | Customer/admin/adapter | Project, amount, currency, external reference policy | Ledger lines only after backend confirmation policy | current terminal: `confirmed`; proposed terminals: `confirmed`, `expired`, `failed`, `refunded` |
| Invoice | IMPLEMENTED: `issued`; PROPOSED: `draft -> issued -> paid/void/disputed/refunded` | Backend/admin | Usage ledger, reservation, pricing snapshot, sufficient balance | Financial ledger transaction | `paid`, `void`, `refunded` |
| Payout | PARTIAL: `held/approved` creation; PROPOSED: `requested -> held -> approved -> submitted -> paid/failed/cancelled` | Admin/backend/adapter | Provider payable balance, KYC/tax, hold policy, payout account | Payout clearing ledger lines, external adapter call | `paid`, `failed`, `cancelled` |

## Error Matrix

Real envelope from `crates/burd-control-plane/src/error.rs`:

```json
{
  "error": {
    "code": "invalid_request",
    "message": "request field failed validation",
    "request_id": "req_example",
    "retry_after_seconds": null,
    "details": {}
  }
}
```

| HTTP | Current code examples | Contract rule |
| --- | --- | --- |
| 400 | `invalid_request`, plus domain invalid mappings | Malformed JSON, invalid field, invalid timestamp, bad id format, invalid enum-like string |
| 401 | `unauthorized`, `signature_invalid` | Missing/invalid bearer or invalid Ed25519/signature proof |
| 403 | `forbidden`, `revoked`, `policy_blocked` | Authenticated actor lacks scope/role, credential/resource was revoked, or backend policy blocks action |
| 404 | `not_found` | Record does not exist or is not visible to actor scope |
| 409 | `conflict`, `idempotency_conflict`, `nonce_reused` | State conflict, stale assignment, reused idempotency key with different payload, nonce replay |
| 410 | `expired` | Current mapping for expired enrollment, nonce, credential, remote session, or proof challenge where helpers return `Expired` |
| 422 | PROPOSED validation split from 400 | Semantic request valid JSON but impossible requirements, unsupported workload contract, incompatible capability |
| 429 | `rate_limited` | Rate limiter exceeded; include `retry_after_seconds` |
| 5xx | `database_unavailable`, `internal` | Dependency unavailable or redacted internal failure; never leak secrets |

Do not invent endpoint-specific implementations where the current route maps a
domain error through shared helper functions. New public compute endpoints must
document exact status/code pairs in OpenAPI and tests.

## OpenAPI Parity Strategy

Current OpenAPI is manually maintained in `crates/burd-control-plane/src/openapi.rs`.
Required parity gate:

- every request/response serde struct in `crates/burd-protocol/src` used by an
  HTTP route has a matching OpenAPI schema;
- route paths and methods match `router()` in `crates/burd-control-plane/src/http.rs`;
- auth schemes and required scopes match route extractors/helpers;
- `Idempotency-Key` appears only where the implementation requires it;
- every error response references `ErrorEnvelope` and includes realistic examples;
- golden JSON fixtures cover request, response, duplicate replay, idempotency
  conflict, unauthorized, forbidden, not found, conflict, rate limit, and
  database unavailable where applicable;
- CI compares serde serialization fixtures, OpenAPI schemas, and route inventory;
- generated or checked OpenAPI must fail when a serde field, enum, schema version,
  route, auth scope, or error code drifts.

## /v1 Breaking Policy

- Do not remove required fields from existing `/v1` responses without a new
  version or compatibility window.
- Do not add required fields to customer request bodies inside stable `/v1`
  without a negotiated feature flag or new endpoint version.
- Physical admin/provider contracts may evolve faster while no production agent
  compatibility promise exists, but breaking changes must be documented.
- Public customer compute must be introduced as new workload/placement routes
  instead of changing customer clients to call physical `/v1/jobs`.
- Schema constants must be bumped when canonical JSON, required fields,
  authority, signature inputs, or state-machine semantics change.
- Deprecation policy must include announce date, final accept date, final response
  date, replacement route/schema, and migration path.

## Single-GPU v1 Definition of Done

- Customer can create a workload without sending `provider_id`, `device_id`,
  `session_id`, or `gpu_uuid`.
- Customer can declare compute requirements without selecting raw provider
  hardware.
- Customer artifact ingress exists with backend-owned upload targets, validation,
  hashes, size limits, redaction, and expiry.
- Backend creates an immutable pricing snapshot before billable placement or
  reservation consumption.
- Backend placement selects exactly one verified GPU from current supply using
  listing, runtime admission, trust, workload eligibility, region, price,
  quota, and active load.
- Backend creates internal physical `ComputeJob` only after placement authority.
- Every physical execution has one `JobAttempt` and one exact `Lease`.
- Assignment, acceptance, active cancellation, and result submission remain bound
  to exact `job_id + lease_id + provider_id + device_id + session_id + gpu_uuid`.
- Customer status/events are projected and sanitized from job/attempt/event/artifact
  state.
- Usage ledger entries bind workload, reservation, pricing snapshot, job attempt,
  job, lease, provider, device, session, and GPU.
- Invoice settlement uses immutable pricing snapshot, not current mutable listing
  price.
- Financial ledger remains append-only with compensating entries for corrections.
- Pix/payment and payout adapters cannot mutate ledger directly.
- Human auth/RBAC separates customer, provider, admin, support, billing, and
  auditor authority.
- OpenAPI, serde, JSON fixtures, auth scopes, error envelopes, idempotency, and
  route inventory pass parity checks.
- `/v1` version/capability negotiation is documented and tested.
- No local/mock/diagnostic state is presented as production remote verification.
- No Windows/NVIDIA policy gate is removed without durable physical evidence.
- No production-ready claim is made until remaining runtime, financial,
  operational, and security gates are complete.

## Follow-up PR Sequence

1. `BN-CONTRACTS-01: customer workload and requirements contracts`
   - Add `Workload`, `ComputeRequirements`, public workload create/read/cancel
     contracts, scopes, idempotency, and OpenAPI fixtures.
2. `BN-CONTRACTS-02: backend placement contract`
   - Add `Placement` resource that chooses supply from listings, runtime
     admission, trust, policy, quota, and price without customer physical IDs.
3. `BN-CONTRACTS-03: job attempts and customer status projection`
   - Add `JobAttempt`, workload-to-job binding, retry semantics, and sanitized
     customer event/status projection.
4. `BN-CONTRACTS-04: immutable pricing snapshot and usage linkage`
   - Add `PricingSnapshot` and bind reservation/workload/placement/job attempt
     to usage and invoices.
5. `BN-CONTRACTS-05: human auth and RBAC hardening`
   - Add explicit human auth/RBAC roles, support/admin actor IDs, and scope
     tests.
6. `BN-CONTRACTS-06: payment and payout adapter boundary`
   - Add reconciliation transition contracts for Pix and payouts without moving
     ledger authority into an external provider.
7. `BN-CONTRACTS-07: OpenAPI parity and version negotiation`
   - Add schema/route/idempotency/error golden fixtures and capability negotiation
     gates.
