# BN-16 - Marketplace Registry And Listings

BN-16 adds the first backend-owned marketplace listing registry. It does not add customer reservations, billing, Pix, payouts, automatic provider pricing, or financial settlement.

## Scope

BN-16 introduces:

- `marketplace_listings`, a materialized backend listing table;
- `POST /v1/marketplace/listings/sweep`, an admin sweep that recalculates listings from backend-owned signals;
- `GET /v1/marketplace/listings`, a listing read endpoint that returns published/limited listings by default;
- `GET /v1/providers/{provider_id}/marketplace-listings`, an admin provider listing inspection endpoint;
- protocol records for listing status, current status, verified GPU/VRAM flags, trust, reliability, network, proof freshness, benchmark evidence, price placeholders, availability window, and source hash.

## Authority Rule

A provider does not publish itself to the marketplace. The backend derives marketplace listing state from existing control-plane records:

- provider and device registry status;
- latest remote session status;
- backend workload eligibility;
- backend trust and reliability state;
- backend verification state and last verified timestamp;
- regional network state;
- signed benchmark result state;
- active scheduler leases.

GPU and VRAM are only marked verified when the listing can bind backend proof state and a succeeded benchmark to the observed GPU UUID. Observed but insufficient evidence stays visible as `backend_observed_unverified`, not as verified marketplace inventory.

## Listing Status

`status` is the marketplace publication status:

- `published`: eligible, verified, and marketplace-visible;
- `limited`: eligible with backend limits and marketplace-visible;
- `verification_required`: observed but not sufficiently verified for marketplace publication;
- `temporarily_unavailable`: verified but not currently available;
- `blocked`: provider/device/policy state blocks listing publication.

`current_status` is the operational state customers or admins can scan:

- `available`;
- `reserved`;
- `degraded`;
- `offline`;
- `blocked`;
- or the non-published marketplace status when more specific availability is not valid.

## Pricing Boundary

BN-16 stores price fields but leaves them empty with `price_source = not_configured_bn16`. Provider pricing, reservations, credits, billing, Pix, payouts, invoice logic, and financial ledgers remain BN-17/BN-18 work.

## Availability Boundary

BN-16 reports a session-bound availability window and active lease count. It does not create reservations and does not allocate jobs. Scheduler leases remain owned by BN-14 job control.

## Trust Boundary

Marketplace listings are snapshots. They do not replace the source-of-truth tables. When trust, proof, benchmark, network, workload eligibility, session, or lease state changes, an admin sweep recalculates the marketplace registry.
