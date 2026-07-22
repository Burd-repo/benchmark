# BN-17 - Customer Accounts And Reservations

BN-17 adds the first customer-side control-plane model. It creates customer organizations, human users, projects, scoped customer API keys, project quotas, a non-settlement credit ledger, marketplace reservations, usage views, and customer audit events.

This is not billing. It does not charge customers, pay providers, settle Pix, issue invoices, or price listings. BN-17 reserves backend-published inventory and records customer/accounting state needed before BN-18 can add a financial ledger.

## Scope

- human customer identity records in the existing `users` table;
- `organizations` with optional owner membership;
- `projects` under organizations;
- backend-enforced project quotas;
- scoped customer API keys returned once and stored only as hashes;
- append-only customer credit ledger entries for grants, adjustments, reservation holds, and releases;
- marketplace reservations against backend-derived BN-16 listings;
- customer usage summaries for reservation counts, reserved GPU seconds, and credit balance;
- customer audit events separated from provider identity and global audit events.

## Database

Migration `0016_customer_accounts_reservations` adds:

- `organizations`
- `organization_users`
- `projects`
- `project_quotas`
- `customer_api_keys`
- `customer_credit_ledger_entries`
- `marketplace_reservations`
- `customer_audit_events`

`customer_credit_ledger_entries` is append-only through a trigger. Reservation hold/release entries are unique per reservation and entry type, so retry paths cannot duplicate the zero-value reservation ledger markers. Active reservations are unique per `listing_id`, preventing two customers from reserving the same published listing at the same time.

## API

Admin endpoints:

- `POST /v1/customer/users`
- `POST /v1/customer/organizations`
- `GET /v1/customer/organizations/{organization_id}`
- `POST /v1/customer/organizations/{organization_id}/projects`
- `GET /v1/customer/organizations/{organization_id}/audit-events`
- `POST /v1/customer/projects/{project_id}/quotas`
- `POST /v1/customer/projects/{project_id}/api-keys`
- `POST /v1/customer/projects/{project_id}/credits`

Customer API-key endpoints:

- `GET /v1/customer/projects/{project_id}/reservations`
- `POST /v1/customer/projects/{project_id}/reservations`
- `GET /v1/customer/projects/{project_id}/usage`
- `POST /v1/customer/reservations/{reservation_id}/cancel`

Reservation creation requires `Authorization: Bearer <customer_api_key>` and `Idempotency-Key`. Admin credit grants also require `Idempotency-Key` so retries replay the stored credit ledger response instead of appending duplicate credits.

## Reservation Rules

A reservation can be created only when:

- the customer API key is active and scoped to the project or organization;
- the key contains `reservations:write`;
- the project and organization are active;
- project quota allows the requested reservation duration;
- the marketplace listing status is `published` or `limited`;
- the listing current status is `available` or `degraded`;
- the optional request `workload_type` matches the listing workload type.

The backend records reservation hold/release entries with zero credit movement because BN-17 has no marketplace pricing or billing settlement. Release entries are recorded both for explicit cancellation and for backend-expired reservations.

## Authority Boundaries

- Customer API keys are separate from provider device credentials.
- Human identity is separate from provider identity.
- Project/reservation identity is separate from workload/job identity.
- Providers cannot create customers, projects, reservations, credits, or audit events.
- Customers cannot self-mark provider inventory verified, available, or priced.
- Marketplace listings remain backend-derived from BN-16 state.

## Non-Goals

BN-17 does not implement:

- checkout;
- marketplace UI;
- provider-set pricing;
- job submission from customer reservations;
- provider-side execution;
- double-entry financial ledger;
- billing, Pix, payouts, invoices, refunds, disputes, or taxes;
- SLA contracts or provider payment obligations.

BN-18 is the first phase intended to add billing, Pix/payment provider adapters, payouts, and financial settlement logic.