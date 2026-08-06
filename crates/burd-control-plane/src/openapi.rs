const START_ENROLLMENT_REQUEST_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/start-enrollment-request.json");
const START_ENROLLMENT_RESPONSE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/start-enrollment-response.json");
const ENROLLMENT_PROOF_REQUEST_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/enrollment-proof-request.json");
const ENROLLMENT_PROOF_RESPONSE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/enrollment-proof-response.json");
const START_REMOTE_SESSION_REQUEST_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/start-remote-session-request.json");
const START_REMOTE_SESSION_RESPONSE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/start-remote-session-response.json");
const HEARTBEAT_CONTROL_MESSAGE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/heartbeat-control-message.json");
const HEARTBEAT_RECEIPT_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/heartbeat-receipt.json");
const SUBMIT_EVIDENCE_REQUEST_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/submit-evidence-request.json");
const SUBMIT_EVIDENCE_RESPONSE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/submit-evidence-response.json");
const ISSUE_PROOF_CHALLENGE_REQUEST_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/issue-proof-challenge-request.json");
const ISSUE_PROOF_CHALLENGE_RESPONSE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/issue-proof-challenge-response.json");
const SIGNED_PROOF_CAPABILITY_RESPONSE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/signed-proof-capability-response.json");
const SUBMIT_PROOF_CHALLENGE_RESPONSE_EXAMPLE_JSON: &str =
    include_str!("../../../docs/examples/control-plane/submit-proof-challenge-response.json");
pub fn document() -> serde_json::Value {
    let mut document = serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Burd Control Plane API",
            "version": "v1",
            "description": "BN-21 control plane API for provider identity, remote sessions, signed GPU telemetry, remote evidence registry, active proof-of-capability challenges, recurring/risk-based verification state, regional network probes, global trust/antifraud state, versioned benchmark profiles, signed benchmark results, backend-owned workload policies, workload eligibility state, first job API/data-plane grants, scheduler-issued job leases, usage metering ledger receipts, marketplace registry listings, customer accounts, project quotas, customer API keys, credits, marketplace reservations, customer usage views, billing price book, Pix payment intents, financial ledger, invoices, customer balances, provider payable balances, payout accounts, provider payouts, customer audit logs, observability metrics, SLO snapshot, signed security posture registry, security hardening policy, attestation posture metadata, outbound WebSocket control channels, revocation, health, readiness, and audit-backed persistence."
        },
        "components": {
            "securitySchemes": {
                "adminBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Bootstrap admin credential configured by BURD_CONTROL_ADMIN_TOKEN. Send only as Authorization: Bearer; never in URLs."
                },
                "deviceBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Short-lived device credential issued after enrollment proof. Send only as Authorization: Bearer; session resume token stays in x-burd-session-token."
                },
                "customerBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Customer API key token returned once by the project API-key endpoint. Send only as Authorization: Bearer; never in URLs."
                }
            },
            "parameters": {
                "IdempotencyKey": {
                    "name": "Idempotency-Key",
                    "in": "header",
                    "required": true,
                    "description": "Required only on mutating endpoints that explicitly list this parameter. Reusing the same key with the same canonical JSON body replays the stored response; reusing it with a different body returns 409 idempotency_conflict.",
                    "schema": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[!-~]+$" }
                }
            },
            "schemas": {
                "ErrorEnvelope": {
                    "type": "object",
                    "required": ["error"],
                    "properties": {
                        "error": {
                            "type": "object",
                            "required": ["code", "message", "request_id", "retry_after_seconds", "details"],
                            "properties": {
                                "code": {
                                    "type": "string",
                                    "enum": ["invalid_request", "unauthorized", "forbidden", "not_found", "conflict", "idempotency_conflict", "rate_limited", "expired", "revoked", "signature_invalid", "nonce_reused", "policy_blocked", "database_unavailable", "internal"]
                                },
                                "message": { "type": "string" },
                                "request_id": { "type": "string" },
                                "retry_after_seconds": { "type": ["integer", "null"], "minimum": 0 },
                                "details": { "type": "object", "additionalProperties": true }
                            }
                        }
                    }
                }
            },
            "responses": {
                "InvalidRequest": {
                    "description": "request validation failed",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "invalid_request": { "value": { "error": { "code": "invalid_request", "message": "request field failed validation", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } } } } }
                },
                "Unauthorized": {
                    "description": "Bearer credential is missing, malformed, invalid, expired, or lacks the required scope",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "unauthorized": { "value": { "error": { "code": "unauthorized", "message": "Authorization: Bearer credential is required", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } } } } }
                },
                "NotFound": {
                    "description": "requested backend record was not found",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "not_found": { "value": { "error": { "code": "not_found", "message": "record not found", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } } } } }
                },
                "Conflict": {
                    "description": "request conflicts with backend-owned state or policy",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "conflict": { "value": { "error": { "code": "conflict", "message": "operation conflicts with backend-owned state", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } } } } }
                },
                "IdempotencyConflict": {
                    "description": "Idempotency-Key was reused with a different canonical JSON body",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "idempotency_conflict": { "value": { "error": { "code": "idempotency_conflict", "message": "idempotency key was reused with a different request body", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } } } } }
                },
                "DatabaseUnavailable": {
                    "description": "database dependency is unavailable; source details are redacted",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "database_unavailable": { "value": { "error": { "code": "database_unavailable", "message": "database unavailable", "request_id": "req_example", "retry_after_seconds": null, "details": { "reason": "database_unavailable" } } } } } } }
                }
            },
            "examples": {
                "BillingInsufficientBalance": { "value": { "error": { "code": "conflict", "message": "project customer balance is insufficient for billing settlement", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } },
                "BillingUsageAlreadyInvoiced": { "value": { "error": { "code": "conflict", "message": "usage ledger entry has already been billed for another reservation", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } },
                "PayoutPolicyBlocked": { "value": { "error": { "code": "conflict", "message": "provider KYC and tax status must be verified before payout", "request_id": "req_example", "retry_after_seconds": null, "details": {} } } }
            }
        },
        "paths": {
            "/health": {
                "get": {
                    "summary": "Liveness check",
                    "responses": { "200": { "description": "service is alive" } }
                }
            },
            "/ready": {
                "get": {
                    "summary": "Readiness check including PostgreSQL and migrations",
                    "responses": {
                        "200": { "description": "service is ready" },
                        "503": { "description": "database or migrations unavailable" }
                    }
                }
            },
            "/openapi.json": {
                "get": {
                    "summary": "OpenAPI document",
                    "responses": { "200": { "description": "OpenAPI JSON" } }
                }
            },
            "/metrics": {
                "get": {
                    "summary": "Prometheus-compatible control-plane metrics",
                    "responses": { "200": { "description": "text/plain Prometheus metrics" } }
                }
            },
            "/v1/observability/snapshot": {
                "get": {
                    "summary": "Admin control-plane observability snapshot",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "HTTP, background task, and SLO state" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/security/policy": {
                "get": {
                    "summary": "Read configured security hardening policy",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "security posture policy flags and accepted modes" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers": {
                "post": {
                    "summary": "Create a provider registry record",
                    "security": [{ "adminBearer": [] }],
                    "parameters": [
                        {
                            "name": "Idempotency-Key",
                            "in": "header",
                            "required": true,
                            "schema": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[!-~]+$" }
                        }
                    ],
                    "responses": {
                        "201": { "description": "provider created as unregistered" },
                        "401": { "description": "admin credential missing or invalid" },
                        "409": { "description": "idempotency conflict" }
                    }
                }
            },
            "/v1/providers/{provider_id}": {
                "get": {
                    "summary": "Fetch a provider registry record",
                    "parameters": [{
                        "name": "provider_id",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }],
                    "responses": {
                        "200": { "description": "provider found" },
                        "404": { "description": "provider not found" }
                    }
                }
            },
            "/v1/providers/{provider_id}/enrollment-tokens": {
                "post": {
                    "summary": "Issue one short-lived enrollment token",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "201": { "description": "token returned once" },
                        "401": { "description": "admin credential missing or invalid" },
                        "404": { "description": "provider not found" }
                    }
                }
            },
            "/v1/enrollments": {
                "post": {
                    "summary": "Consume an enrollment token and issue a proof nonce",
                    "responses": {
                        "202": { "description": "enrollment pending Ed25519 proof" },
                        "401": { "description": "token invalid" },
                        "403": { "description": "token revoked" },
                        "410": { "description": "token expired" }
                    }
                }
            },
            "/v1/enrollments/{enrollment_id}/proof": {
                "post": {
                    "summary": "Complete enrollment with Ed25519 possession proof",
                    "responses": {
                        "201": { "description": "device and short-lived credential created" },
                        "401": { "description": "signature invalid" },
                        "409": { "description": "nonce reused or identity conflict" },
                        "410": { "description": "proof expired" }
                    }
                }
            },
            "/v1/providers/{provider_id}/devices": {
                "get": {
                    "summary": "List provider devices and active public key IDs",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "200": { "description": "device list" } }
                }
            },
            "/v1/devices/{device_id}/credentials": {
                "post": {
                    "summary": "Rotate a short-lived device credential",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "201": { "description": "new credential returned once" },
                        "401": { "description": "credential invalid or expired" },
                        "403": { "description": "device revoked" }
                    }
                }
            },
            "/v1/devices/{device_id}/key-rotations": {
                "post": {
                    "summary": "Issue a nonce for a new Ed25519 device key",
                    "security": [{ "deviceBearer": [] }],
                    "responses": { "202": { "description": "rotation pending proof by new key" } }
                }
            },
            "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof": {
                "post": {
                    "summary": "Activate a new Ed25519 key and revoke the previous key",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "new key active" },
                        "401": { "description": "credential or signature invalid" },
                        "409": { "description": "nonce reused or active key changed" }
                    }
                }
            },
            "/v1/devices/{device_id}/revoke": {
                "post": {
                    "summary": "Revoke a device, keys, credentials, and pending rotations",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "device revoked" },
                        "404": { "description": "device not found" }
                    }
                }
            },
            "/v1/sessions": {
                "post": {
                    "summary": "Start or resume a remote provider session",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "201": { "description": "session authorized; resume token returned" },
                        "401": { "description": "device credential invalid" },
                        "409": { "description": "duplicate active session or invalid resume" }
                    }
                }
            },
            "/v1/sessions/{session_id}": {
                "get": {
                    "summary": "Read backend-authoritative remote session state",
                    "security": [{ "deviceBearer": [] }],
                    "responses": { "200": { "description": "session state" } }
                }
            },
            "/v1/sessions/{session_id}/control": {
                "get": {
                    "summary": "Upgrade to the authenticated outbound WebSocket control channel",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "101": { "description": "WebSocket control channel established" },
                        "409": { "description": "duplicate control channel" }
                    }
                }
            },
            "/v1/sessions/{session_id}/heartbeats": {
                "post": {
                    "summary": "Submit a sequenced heartbeat over HTTP fallback",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "heartbeat observed by server" },
                        "409": { "description": "duplicate or stale sequence" }
                    }
                }
            },
            "/v1/sessions/{session_id}/security-posture": {
                "post": {
                    "summary": "Submit signed agent security posture and attestation metadata",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "201": { "description": "security posture verified, classified, and stored" },
                        "200": { "description": "duplicate posture hash returned from registry" },
                        "400": { "description": "invalid schema, hash, signature, binding, or policy metadata" },
                        "401": { "description": "device, session, key, or signature invalid" }
                    }
                }
            },
            "/v1/sessions/{session_id}/gpu-inventory": {
                "post": {
                    "summary": "Submit signed device GPU inventory snapshot",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "201": { "description": "GPU inventory verified and stored" },
                        "200": { "description": "duplicate inventory hash returned from registry" },
                        "400": { "description": "invalid schema, hash, signature, or inventory payload" },
                        "401": { "description": "device, session, key, or signature invalid" }
                    }
                }
            },
            "/v1/sessions/{session_id}/revoke": {
                "post": {
                    "summary": "Revoke a remote session and signal its active channel",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "200": { "description": "session revoked" } }
                }
            },
            "/v1/sessions/{session_id}/telemetry-batches": {
                "post": {
                    "summary": "Ingest a signed, sequenced GPU telemetry batch",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "telemetry batch verified and persisted" },
                        "400": { "description": "invalid metrics, hash, schema, or timestamps" },
                        "401": { "description": "device, session, key, or signature invalid" },
                        "409": { "description": "sequence, fingerprint, or frequency conflict" }
                    }
                }
            },
            "/v1/sessions/{session_id}/telemetry/latest": {
                "get": {
                    "summary": "Read the latest server-verified GPU telemetry batch",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "latest verified telemetry samples" },
                        "404": { "description": "no telemetry has been accepted" }
                    }
                }
            },
            "/v1/sessions/{session_id}/evidence-records": {
                "post": {
                    "summary": "Submit a signed report envelope for backend evidence verification",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "201": { "description": "evidence verified, stored, and indexed" },
                        "200": { "description": "duplicate evidence hash returned from registry" },
                        "400": { "description": "invalid hash, canonicalization, fingerprint, metadata, or freshness" },
                        "401": { "description": "device, session, key, provider binding, or signature invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/evidence-records": {
                "get": {
                    "summary": "List remote evidence registry records for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "evidence metadata list" },
                        "404": { "description": "provider not found" }
                    }
                }
            },
            "/v1/providers/{provider_id}/security-postures": {
                "get": {
                    "summary": "List signed security posture records for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "security posture records and backend verification state" },
                        "404": { "description": "provider not found" }
                    }
                }
            },
            "/v1/providers/{provider_id}/gpu-inventory": {
                "get": {
                    "summary": "List device GPU inventory records for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "GPU inventory records returned" },
                        "404": { "description": "provider not found" }
                    }
                }
            },
            "/v1/evidence-records/{evidence_id}": {
                "get": {
                    "summary": "Read one remote evidence registry record",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "evidence metadata and backend verification state" },
                        "404": { "description": "evidence record not found" }
                    }
                }
            },
            "/v1/evidence-records/{evidence_id}/revoke": {
                "post": {
                    "summary": "Revoke a remote evidence registry record",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "evidence revoked" },
                        "404": { "description": "evidence record not found" }
                    }
                }
            },
            "/v1/network-probes/observations": {
                "post": {
                    "summary": "Submit a trusted regional network probe observation",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "201": { "description": "probe observation stored and provider network state recalculated" },
                        "200": { "description": "duplicate probe observation returned without changing score history" },
                        "400": { "description": "invalid probe metrics, timestamps, or metadata" },
                        "401": { "description": "admin/probe credential missing or invalid" },
                        "404": { "description": "provider, device, or session not found" }
                    }
                }
            },
            "/v1/providers/{provider_id}/network-probes": {
                "get": {
                    "summary": "List trusted regional network probe observations for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "network probe observation history returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/network-state": {
                "get": {
                    "summary": "List backend-calculated network state for provider devices",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "remote network score, regional reachability, and effective score returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/benchmark-profiles": {
                "get": {
                    "summary": "List versioned benchmark workload profiles",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "benchmark profile registry returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                },
                "post": {
                    "summary": "Create or update a versioned benchmark workload profile",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "201": { "description": "benchmark profile created or updated" },
                        "400": { "description": "invalid profile, thresholds, digest, or redaction" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/sessions/{session_id}/benchmark-results": {
                "post": {
                    "summary": "Submit a signed benchmark result for backend verification",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "201": { "description": "benchmark result verified and stored" },
                        "200": { "description": "duplicate result hash returned without changing history" },
                        "400": { "description": "invalid result hash, schema, profile binding, metrics, or timestamps" },
                        "401": { "description": "device, session, key, or signature invalid" },
                        "409": { "description": "session state, fingerprint, or run id conflict" }
                    }
                }
            },
            "/v1/providers/{provider_id}/benchmark-results": {
                "get": {
                    "summary": "List backend-verified benchmark results for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "signed benchmark result history returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/workload-policies": {
                "get": {
                    "summary": "List backend-owned workload eligibility policies",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "workload policy registry returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                },
                "post": {
                    "summary": "Create or update a backend-owned workload eligibility policy",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "201": { "description": "workload policy created or updated" },
                        "400": { "description": "invalid policy, requirement, threshold, or redaction" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/workload-eligibility/sweep": {
                "post": {
                    "summary": "Run one backend workload eligibility sweep",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "202": { "description": "provider-device workload eligibility states recalculated" },
                        "400": { "description": "invalid sweep request" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/workload-eligibility": {
                "get": {
                    "summary": "List backend-calculated workload eligibility states for provider devices",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider workload eligibility states returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/marketplace/listings": {
                "get": {
                    "summary": "List backend-published marketplace provider listings",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "published marketplace listings returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/marketplace/listings/sweep": {
                "post": {
                    "summary": "Run one backend marketplace listing registry sweep",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "202": { "description": "marketplace listings recalculated from backend-owned signals" },
                        "400": { "description": "invalid sweep request" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/marketplace/listings/{listing_id}/price": {
                "post": {
                    "summary": "Configure the active billing price for a marketplace listing",
                    "description": "Admin-only BN-18 price-book write. This updates backend-owned listing price fields; providers cannot self-report authoritative billing prices.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "price stored and listing price fields updated" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" }
                    }
                }
            },
            "/v1/billing/projects/{project_id}/pix/payment-intents": {
                "post": {
                    "summary": "Create a customer Pix payment intent without ledger movement until confirmation",
                    "description": "Customer-scoped write requiring `billing:write` and `Idempotency-Key`. Creation records the intent only; it does not credit `customer_balance` until the admin/adapter confirmation endpoint succeeds.",
                    "security": [{ "customerBearer": [] }],
                    "parameters": [{ "$ref": "#/components/parameters/IdempotencyKey" }],
                    "responses": {
                        "201": { "description": "Pix intent created or idempotently replayed from the stored response" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" },
                        "409": { "$ref": "#/components/responses/IdempotencyConflict" }
                    }
                }
            },
            "/v1/billing/pix/payment-intents/{payment_intent_id}/confirm": {
                "post": {
                    "summary": "Confirm a Pix payment intent and credit customer balance through double-entry ledger lines",
                    "description": "Admin/adapter boundary. Exact duplicate confirmations replay as `duplicate=true`; conflicting provider, external reference, or paid_at evidence returns 409 `conflict` and appends no ledger lines.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "payment intent confirmed or exact duplicate returned" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" },
                        "409": { "$ref": "#/components/responses/Conflict" }
                    }
                }
            },
            "/v1/billing/projects/{project_id}/balance": {
                "get": {
                    "summary": "Read customer project financial balances",
                    "description": "Customer-scoped read requiring `billing:read`; balances are derived from append-only `financial_ledger_lines`.",
                    "security": [{ "customerBearer": [] }],
                    "responses": {
                        "200": { "description": "project balances returned" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" }
                    }
                }
            },
            "/v1/billing/projects/{project_id}/ledger": {
                "get": {
                    "summary": "List customer project financial ledger lines",
                    "description": "Customer-scoped read requiring `billing:read`; returned rows are append-only accounting records, not mutable balances.",
                    "security": [{ "customerBearer": [] }],
                    "responses": {
                        "200": { "description": "project ledger lines returned" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" }
                    }
                }
            },
            "/v1/billing/reservations/{reservation_id}/settle": {
                "post": {
                    "summary": "Settle metered reservation usage into invoice and double-entry financial ledger",
                    "description": "Admin-only settlement over a BN-17 reservation, BN-15 usage entry, active price book row, and sufficient confirmed project balance. Same reservation/usage replay returns the existing invoice; cross-reservation usage reuse returns 409 `conflict`.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "invoice issued or existing same-reservation invoice returned" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" },
                        "409": {
                            "description": "usage, price, balance, binding, or reservation state cannot be billed",
                            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "insufficient_balance": { "$ref": "#/components/examples/BillingInsufficientBalance" }, "usage_already_invoiced": { "$ref": "#/components/examples/BillingUsageAlreadyInvoiced" } } } }
                        }
                    }
                }
            },
            "/v1/billing/invoices/{invoice_id}": {
                "get": {
                    "summary": "Read a billing invoice",
                    "description": "Admin-only read of backend-derived invoice metadata. Invoice totals are calculated by the control plane, not supplied by customers or providers.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "invoice returned" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" }
                    }
                }
            },
            "/v1/billing/providers/{provider_id}/balance": {
                "get": {
                    "summary": "Read provider payable balances",
                    "description": "Admin-only read derived from `financial_ledger_lines`; provider-submitted balances are never accepted.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider balances returned" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" }
                    }
                }
            },
            "/v1/billing/providers/{provider_id}/ledger": {
                "get": {
                    "summary": "List provider financial ledger lines",
                    "description": "Admin-only append-only ledger read for provider payable and payout-clearing inspection.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider ledger lines returned" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" }
                    }
                }
            },
            "/v1/billing/providers/{provider_id}/payout-account": {
                "post": {
                    "summary": "Create or update a provider Pix payout account with KYC and tax status",
                    "description": "Admin-only account metadata write. The API stores hashed Pix key material and a masked suffix; it does not store raw Pix keys or execute payouts.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "payout account upserted" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" }
                    }
                }
            },
            "/v1/billing/providers/{provider_id}/payouts": {
                "post": {
                    "summary": "Create a provider payout from payable balance subject to minimum payout and hold policy",
                    "description": "Admin-only accounting reservation. Payout creation moves `provider_payable` into `provider_payout_clearing`; it does not call a bank, mark the payout paid, or release funds externally.",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "201": { "description": "payout accounting record created" },
                        "400": { "$ref": "#/components/responses/InvalidRequest" },
                        "401": { "$ref": "#/components/responses/Unauthorized" },
                        "404": { "$ref": "#/components/responses/NotFound" },
                        "409": {
                            "description": "provider balance, KYC, tax, payout account, or minimum payout policy blocks payout",
                            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ErrorEnvelope" }, "examples": { "policy_blocked": { "$ref": "#/components/examples/PayoutPolicyBlocked" } } } }
                        }
                    }
                }
            },
            "/v1/customer/users": {
                "post": {
                    "summary": "Create a human customer identity record",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "201": { "description": "customer user created" }, "401": { "description": "admin credential missing or invalid" } }
                }
            },
            "/v1/customer/organizations": {
                "post": {
                    "summary": "Create a customer organization and optional owner membership",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "201": { "description": "organization created" }, "404": { "description": "owner user not found" } }
                }
            },
            "/v1/customer/organizations/{organization_id}": {
                "get": {
                    "summary": "Read a customer organization",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "200": { "description": "organization returned" }, "404": { "description": "organization not found" } }
                }
            },
            "/v1/customer/organizations/{organization_id}/projects": {
                "post": {
                    "summary": "Create a customer project with default reservation quota",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "201": { "description": "project created" }, "409": { "description": "organization is not active" } }
                }
            },
            "/v1/customer/organizations/{organization_id}/audit-events": {
                "get": {
                    "summary": "List customer audit log events for an organization",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "200": { "description": "customer audit events returned" } }
                }
            },
            "/v1/customer/projects/{project_id}/quotas": {
                "post": {
                    "summary": "Upsert backend-enforced customer project reservation quota",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "200": { "description": "quota updated" }, "400": { "description": "invalid quota" } }
                }
            },
            "/v1/customer/projects/{project_id}/api-keys": {
                "post": {
                    "summary": "Create a scoped customer API key for one project",
                    "security": [{ "adminBearer": [] }],
                    "responses": { "201": { "description": "API key token returned once" }, "404": { "description": "project not found" } }
                }
            },
            "/v1/customer/projects/{project_id}/credits": {
                "post": {
                    "summary": "Append a non-settlement customer credit ledger entry",
                    "security": [{ "adminBearer": [] }],
                    "parameters": [{ "name": "Idempotency-Key", "in": "header", "required": true, "schema": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[!-~]+$" } }],
                    "responses": { "201": { "description": "credit ledger entry appended" }, "409": { "description": "idempotency conflict" } }
                }
            },
            "/v1/customer/projects/{project_id}/reservations": {
                "get": {
                    "summary": "List project marketplace reservations",
                    "security": [{ "customerBearer": [] }],
                    "responses": { "200": { "description": "reservations returned" }, "401": { "description": "customer API key invalid or scope missing" } }
                },
                "post": {
                    "summary": "Reserve one backend-published marketplace listing for a project",
                    "security": [{ "customerBearer": [] }],
                    "parameters": [{ "name": "Idempotency-Key", "in": "header", "required": true, "schema": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[!-~]+$" } }],
                    "responses": { "201": { "description": "reservation created" }, "409": { "description": "quota exceeded, listing unavailable, or idempotency conflict" } }
                }
            },
            "/v1/customer/projects/{project_id}/usage": {
                "get": {
                    "summary": "Read project reservation usage and credit balance view",
                    "security": [{ "customerBearer": [] }],
                    "responses": { "200": { "description": "usage summary returned" } }
                }
            },
            "/v1/customer/reservations/{reservation_id}/cancel": {
                "post": {
                    "summary": "Cancel a customer marketplace reservation",
                    "security": [{ "customerBearer": [] }],
                    "responses": { "200": { "description": "reservation cancelled or returned as duplicate terminal state" }, "404": { "description": "reservation not found" } }
                }
            },
            "/v1/jobs": {
                "post": {
                    "summary": "Create one backend-authorized compute job for a specific provider session",
                    "security": [{ "adminBearer": [] }],
                    "parameters": [{
                        "name": "Idempotency-Key",
                        "in": "header",
                        "required": true,
                        "schema": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[!-~]+$" }
                    }],
                    "responses": {
                        "201": { "description": "job queued for an online or degraded eligible provider session" },
                        "400": { "description": "invalid template, digest, artifact, parameter, backend, or idempotency key" },
                        "401": { "description": "admin credential missing or invalid" },
                        "404": { "description": "provider, device, or session not found" },
                        "409": { "description": "idempotency conflict, ineligible workload, blocked target, or unavailable session" }
                    }
                }
            },
            "/v1/jobs/{job_id}": {
                "get": {
                    "summary": "Read compute job metadata and status",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "job metadata returned" },
                        "401": { "description": "admin credential missing or invalid" },
                        "404": { "description": "job not found" }
                    }
                }
            },
            "/v1/jobs/{job_id}/usage-ledger": {
                "get": {
                    "summary": "List append-only usage ledger entries for a compute job",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "job usage ledger entries returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/jobs/{job_id}/usage-ledger/finalize": {
                "post": {
                    "summary": "Finalize or replay the backend-derived usage receipt for a terminal job",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "201": { "description": "usage ledger entry appended" },
                        "200": { "description": "existing usage ledger entry returned" },
                        "401": { "description": "admin credential missing or invalid" },
                        "404": { "description": "job not found" },
                        "409": { "description": "job is not terminal" }
                    }
                }
            },
            "/v1/jobs/{job_id}/leases": {
                "get": {
                    "summary": "List scheduler leases for a compute job",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "job lease history returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/jobs/{job_id}/cancel": {
                "post": {
                    "summary": "Cancel a non-terminal compute job",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "job cancelled" },
                        "401": { "description": "admin credential missing or invalid" },
                        "404": { "description": "job not found" },
                        "409": { "description": "terminal jobs cannot be cancelled" }
                    }
                }
            },
            "/v1/providers/{provider_id}/jobs": {
                "get": {
                    "summary": "List compute jobs for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider job metadata returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/marketplace-listings": {
                "get": {
                    "summary": "List marketplace listing registry records for one provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider marketplace listings returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/usage-ledger": {
                "get": {
                    "summary": "List append-only usage ledger entries for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider usage ledger entries returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/leases": {
                "get": {
                    "summary": "List scheduler leases for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider lease history returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/scheduler/run": {
                "post": {
                    "summary": "Run one bounded scheduler pass and offer leases for eligible queued jobs",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "202": { "description": "scheduler pass completed and lease decisions returned" },
                        "400": { "description": "invalid scheduler request" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/sessions/{session_id}/jobs/next": {
                "get": {
                    "summary": "Accept the next offered scheduler lease and fetch its compute job and data-plane grant",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "next job and scoped artifact URLs returned, or no job is available" },
                        "401": { "description": "device, session, key, or credential invalid" },
                        "410": { "description": "session expired" }
                    }
                }
            },
            "/v1/sessions/{session_id}/jobs/{job_id}/accept": {
                "post": {
                    "summary": "Acknowledge a job assignment before provider provisioning",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "job accepted" },
                        "401": { "description": "device or session unauthorized for job" },
                        "404": { "description": "job not found" },
                        "409": { "description": "job is not assigned" }
                    }
                }
            },
            "/v1/sessions/{session_id}/jobs/{job_id}/events": {
                "post": {
                    "summary": "Append a sequenced provider job progress event",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "201": { "description": "event stored and job status updated" },
                        "400": { "description": "invalid sequence, event type, timestamp, progress, or metadata" },
                        "401": { "description": "device or session unauthorized for job" },
                        "409": { "description": "duplicate sequence or terminal job" }
                    }
                }
            },
            "/v1/sessions/{session_id}/jobs/{job_id}/result": {
                "post": {
                    "summary": "Submit final job result metadata and output artifact references",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "job result accepted" },
                        "400": { "description": "invalid final status, artifact, metric, timestamp, or error payload" },
                        "401": { "description": "device or session unauthorized for job" },
                        "409": { "description": "terminal job result cannot be changed" }
                    }
                }
            },
            "/v1/trust/sweep": {
                "post": {
                    "summary": "Run one backend global trust and antifraud sweep",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "202": { "description": "provider-device trust states recalculated and antifraud events recorded" },
                        "400": { "description": "invalid sweep request" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/trust-states": {
                "get": {
                    "summary": "List backend-calculated trust states for provider devices",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "provider trust states returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/antifraud-events": {
                "get": {
                    "summary": "List active backend antifraud events for a provider",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "antifraud event history returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/verification/sweep": {
                "post": {
                    "summary": "Run one recurring/risk-based verification sweep",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "202": { "description": "eligible online sessions evaluated and due challenges issued" },
                        "400": { "description": "invalid sweep request or recurring proof profile is not configured" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/providers/{provider_id}/verification-states": {
                "get": {
                    "summary": "List backend verification policy state for provider devices",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "verification states returned" },
                        "401": { "description": "admin credential missing or invalid" }
                    }
                }
            },
            "/v1/challenges": {
                "post": {
                    "summary": "Issue an active proof-of-capability challenge for an online session",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "201": { "description": "challenge issued with nonce, artifact hash, expiry, and requirements" },
                        "400": { "description": "challenge fields or proof requirements are invalid" },
                        "401": { "description": "admin credential missing or invalid" },
                        "404": { "description": "provider, device, or session not found" },
                        "409": { "description": "session is not online/degraded or fingerprint does not match" }
                    }
                }
            },
            "/v1/challenges/{challenge_id}": {
                "get": {
                    "summary": "Read backend challenge state and verification result",
                    "security": [{ "adminBearer": [] }],
                    "responses": {
                        "200": { "description": "proof challenge record" },
                        "404": { "description": "proof challenge not found" }
                    }
                }
            },
            "/v1/sessions/{session_id}/challenges/next": {
                "get": {
                    "summary": "Fetch the next issued proof-of-capability challenge for a device session",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "challenge acknowledged and returned" },
                        "404": { "description": "no active challenge for session" },
                        "410": { "description": "session expired" }
                    }
                }
            },
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response": {
                "post": {
                    "summary": "Submit a signed proof-of-capability response for backend verification",
                    "security": [{ "deviceBearer": [] }],
                    "responses": {
                        "200": { "description": "response stored with verified or failed status" },
                        "400": { "description": "malformed response or unsupported schema" },
                        "401": { "description": "device, session, key, or signature invalid" },
                        "410": { "description": "challenge expired by server clock" },
                        "409": { "description": "challenge is not accepting responses" }
                    }
                }
            }
        }
    });
    add_bn01_bn11_contracts(&mut document);
    add_jobs_scheduler_reservation_contracts(&mut document);
    add_control_plane_protocol_examples(&mut document);
    document
}

fn add_bn01_bn11_contracts(document: &mut serde_json::Value) {
    {
        let schemas = document["components"]["schemas"]
            .as_object_mut()
            .expect("OpenAPI schemas object");
        insert_structural_schemas(
            schemas,
            &[
                (
                    "HealthResponse",
                    &["status", "service", "version", "environment", "request_id"],
                ),
                (
                    "ReadyResponse",
                    &[
                        "status",
                        "service",
                        "database",
                        "migrations_applied",
                        "migrations_expected",
                        "request_id",
                    ],
                ),
                ("CreateProviderRequest", &[]),
                (
                    "ProviderRecord",
                    &["provider_id", "status", "created_at", "updated_at"],
                ),
                ("ProviderEnvelope", &["request_id", "provider"]),
                (
                    "IssueEnrollmentTokenResponse",
                    &["request_id", "enrollment_token", "expires_at", "max_uses"],
                ),
                (
                    "StartEnrollmentRequest",
                    &[
                        "enrollment_token",
                        "public_key",
                        "key_algorithm",
                        "machine_id",
                        "registration_payload",
                        "hardware_fingerprint",
                        "agent_version",
                        "benchmark_version",
                    ],
                ),
                (
                    "StartEnrollmentResponse",
                    &[
                        "request_id",
                        "enrollment_id",
                        "provider_id",
                        "nonce",
                        "expires_at",
                    ],
                ),
                (
                    "EnrollmentProofRequest",
                    &["nonce", "signature", "public_key", "hardware_fingerprint"],
                ),
                (
                    "EnrollmentProofResponse",
                    &[
                        "request_id",
                        "provider_id",
                        "device_id",
                        "public_key_id",
                        "credential",
                        "credential_expires_at",
                        "status",
                    ],
                ),
                (
                    "DeviceRecord",
                    &[
                        "device_id",
                        "provider_id",
                        "status",
                        "created_at",
                        "updated_at",
                    ],
                ),
                ("ListProviderDevicesResponse", &["request_id", "devices"]),
                (
                    "DeviceCredentialResponse",
                    &[
                        "request_id",
                        "provider_id",
                        "device_id",
                        "credential",
                        "credential_expires_at",
                    ],
                ),
                (
                    "StartKeyRotationRequest",
                    &["new_public_key", "key_algorithm"],
                ),
                (
                    "StartKeyRotationResponse",
                    &[
                        "request_id",
                        "rotation_id",
                        "provider_id",
                        "device_id",
                        "current_public_key_id",
                        "nonce",
                        "expires_at",
                    ],
                ),
                (
                    "KeyRotationProofRequest",
                    &["nonce", "signature", "new_public_key"],
                ),
                (
                    "KeyRotationProofResponse",
                    &[
                        "request_id",
                        "provider_id",
                        "device_id",
                        "public_key_id",
                        "status",
                    ],
                ),
                (
                    "DeviceRevocationResponse",
                    &[
                        "request_id",
                        "provider_id",
                        "device_id",
                        "status",
                        "revoked_at",
                    ],
                ),
                ("RemoteSessionResume", &["session_id", "resume_token"]),
                (
                    "StartRemoteSessionRequest",
                    &[
                        "provider_id",
                        "device_id",
                        "hardware_fingerprint",
                        "agent_version",
                    ],
                ),
                (
                    "StartRemoteSessionResponse",
                    &[
                        "request_id",
                        "session_id",
                        "resume_token",
                        "status",
                        "expires_at",
                        "heartbeat_interval_seconds",
                        "missed_heartbeat_limit",
                        "sequence_start",
                        "telemetry_sequence_start",
                        "control_url",
                    ],
                ),
                (
                    "RemoteSessionRecord",
                    &[
                        "request_id",
                        "session_id",
                        "provider_id",
                        "device_id",
                        "status",
                        "sequence_last",
                        "started_at",
                        "expires_at",
                    ],
                ),
                ("HeartbeatPayload", &["hardware_fingerprint"]),
                (
                    "HeartbeatControlMessage",
                    &[
                        "session_id",
                        "device_id",
                        "sequence",
                        "sent_at",
                        "type",
                        "payload",
                    ],
                ),
                (
                    "HeartbeatReceipt",
                    &[
                        "request_id",
                        "session_id",
                        "sequence_ack",
                        "status",
                        "server_time",
                        "next_heartbeat_seconds",
                    ],
                ),
                (
                    "RemoteSessionRevocationResponse",
                    &["request_id", "session_id", "status", "revoked_at"],
                ),
                (
                    "GpuProcessTelemetry",
                    &["pid", "process_name", "process_kind"],
                ),
                (
                    "GpuTelemetrySample",
                    &[
                        "sample_sequence",
                        "observed_at",
                        "gpu_uuid",
                        "gpu_name",
                        "pci_bus_id",
                        "driver_version",
                        "vram_total_mib",
                        "throttle_reasons",
                        "processes",
                    ],
                ),
                (
                    "TelemetryBatchPayload",
                    &[
                        "schema_version",
                        "provider_id",
                        "device_id",
                        "session_id",
                        "control_sequence",
                        "sample_sequence_start",
                        "sample_sequence_end",
                        "hardware_fingerprint",
                        "collector",
                        "collected_at_start",
                        "collected_at_end",
                        "samples",
                    ],
                ),
                (
                    "SignedTelemetryBatch",
                    &[
                        "payload",
                        "batch_hash",
                        "public_key_id",
                        "signature",
                        "canonicalization_version",
                    ],
                ),
                (
                    "TelemetryBatchControlMessage",
                    &[
                        "session_id",
                        "device_id",
                        "sequence",
                        "sent_at",
                        "type",
                        "payload",
                    ],
                ),
                (
                    "TelemetryBatchReceipt",
                    &[
                        "request_id",
                        "batch_id",
                        "session_id",
                        "control_sequence_ack",
                        "sample_sequence_end",
                        "sample_count",
                        "batch_hash",
                        "status",
                        "server_received_at",
                    ],
                ),
                (
                    "LatestTelemetryResponse",
                    &[
                        "request_id",
                        "session_id",
                        "batch_id",
                        "batch_hash",
                        "server_received_at",
                        "samples",
                    ],
                ),
                (
                    "SignedReportEnvelope",
                    &[
                        "provider_id",
                        "machine_id",
                        "report",
                        "report_hash",
                        "signature",
                        "public_key",
                        "key_algorithm",
                        "signed_at",
                        "canonicalization_version",
                    ],
                ),
                (
                    "EvidenceFreshness",
                    &[
                        "issued_at",
                        "expires_at",
                        "is_expired",
                        "age_seconds",
                        "ttl_seconds",
                    ],
                ),
                ("SubmitEvidenceRequest", &["signed_report"]),
                (
                    "EvidenceVerification",
                    &[
                        "schema_version",
                        "checked_at",
                        "report_hash_valid",
                        "evidence_hash_valid",
                        "signature_valid",
                        "active_key_bound",
                        "provider_bound",
                        "device_bound",
                        "fingerprint_bound",
                        "expired_by_server",
                        "warnings",
                        "errors",
                    ],
                ),
                (
                    "EvidenceRecord",
                    &[
                        "evidence_id",
                        "provider_id",
                        "evidence_type",
                        "canonicalization_version",
                        "evidence_hash",
                        "status",
                        "server_received_at",
                        "verification",
                    ],
                ),
                (
                    "SubmitEvidenceResponse",
                    &["request_id", "duplicate", "evidence"],
                ),
                (
                    "ListEvidenceResponse",
                    &["request_id", "provider_id", "records"],
                ),
                ("RevokeEvidenceRequest", &["reason"]),
                (
                    "RevokeEvidenceResponse",
                    &[
                        "request_id",
                        "evidence_id",
                        "status",
                        "revoked_at",
                        "reason",
                    ],
                ),
                (
                    "IssueProofChallengeRequest",
                    &[
                        "provider_id",
                        "device_id",
                        "session_id",
                        "profile_version",
                        "required_fingerprint",
                        "required_backend",
                        "model_artifact_hash",
                        "prompt_seed",
                        "min_tokens_per_second",
                        "max_ttft_ms",
                    ],
                ),
                (
                    "ProofCapabilityChallenge",
                    &[
                        "schema_version",
                        "challenge_id",
                        "nonce",
                        "provider_id",
                        "device_id",
                        "session_id",
                        "profile_version",
                        "required_fingerprint",
                        "required_backend",
                        "model_artifact_hash",
                        "prompt_seed",
                        "required_proofs",
                        "min_tokens_per_second",
                        "max_ttft_ms",
                        "issued_at",
                        "expires_at",
                    ],
                ),
                (
                    "ProofCapabilityMetrics",
                    &[
                        "cuda_runtime_detected",
                        "backend_proof",
                        "contention_detected",
                    ],
                ),
                (
                    "ProofCapabilityResponsePayload",
                    &[
                        "schema_version",
                        "challenge_id",
                        "nonce",
                        "provider_id",
                        "device_id",
                        "session_id",
                        "profile_version",
                        "hardware_fingerprint",
                        "gpu_uuid",
                        "backend",
                        "model_artifact_hash",
                        "prompt_seed",
                        "driver_version",
                        "metrics",
                        "started_at",
                        "completed_at",
                    ],
                ),
                (
                    "SignedProofCapabilityResponse",
                    &[
                        "payload",
                        "response_hash",
                        "public_key_id",
                        "signature",
                        "canonicalization_version",
                    ],
                ),
                (
                    "ProofChallengeVerification",
                    &[
                        "schema_version",
                        "challenge_id",
                        "checked_at",
                        "response_hash_valid",
                        "signature_valid",
                        "provider_bound",
                        "device_bound",
                        "session_bound",
                        "fingerprint_bound",
                        "gpu_bound",
                        "backend_bound",
                        "artifact_bound",
                        "prompt_bound",
                        "metrics_satisfied",
                        "expired_by_server",
                        "warnings",
                        "errors",
                    ],
                ),
                (
                    "ProofChallengeRecord",
                    &["challenge", "status", "issued_at"],
                ),
                ("IssueProofChallengeResponse", &["request_id", "challenge"]),
                ("NextProofChallengeResponse", &["request_id", "challenge"]),
                (
                    "SubmitProofChallengeResponse",
                    &[
                        "request_id",
                        "challenge_id",
                        "status",
                        "response_hash",
                        "server_received_at",
                        "verification",
                    ],
                ),
                ("RunVerificationSweepRequest", &[]),
                (
                    "VerificationSweepIssuedChallenge",
                    &[
                        "provider_id",
                        "device_id",
                        "session_id",
                        "challenge_id",
                        "reason",
                    ],
                ),
                (
                    "RunVerificationSweepResponse",
                    &["request_id", "evaluated", "issued"],
                ),
                (
                    "VerificationStateRecord",
                    &[
                        "provider_id",
                        "device_id",
                        "status",
                        "policy_version",
                        "risk_score",
                        "success_count",
                        "failure_count",
                        "retry_budget_remaining",
                        "updated_at",
                    ],
                ),
                ("ListVerificationStatesResponse", &["request_id", "states"]),
                (
                    "SubmitNetworkProbeObservationRequest",
                    &[
                        "provider_id",
                        "device_id",
                        "session_id",
                        "probe_id",
                        "probe_region",
                        "observed_at",
                        "sample_count",
                    ],
                ),
                (
                    "NetworkProbeObservationRecord",
                    &[
                        "observation_id",
                        "provider_id",
                        "device_id",
                        "session_id",
                        "probe_id",
                        "probe_region",
                        "schema_version",
                        "observed_at",
                        "server_received_at",
                        "sample_count",
                        "remote_network_score",
                        "status",
                        "warnings",
                        "metadata",
                    ],
                ),
                (
                    "RegionalReachability",
                    &[
                        "probe_region",
                        "status",
                        "remote_network_score",
                        "sample_count",
                        "observed_at",
                    ],
                ),
                (
                    "ProviderNetworkState",
                    &[
                        "provider_id",
                        "device_id",
                        "regional_reachability",
                        "sample_count",
                        "updated_at",
                    ],
                ),
                (
                    "SubmitNetworkProbeObservationResponse",
                    &["request_id", "duplicate", "observation", "network_state"],
                ),
                (
                    "ListNetworkProbeObservationsResponse",
                    &["request_id", "observations"],
                ),
                (
                    "ListProviderNetworkStatesResponse",
                    &["request_id", "states"],
                ),
                ("RunTrustSweepRequest", &[]),
                (
                    "TrustSweepUpdatedState",
                    &[
                        "provider_id",
                        "device_id",
                        "status",
                        "trust_score",
                        "risk_score",
                        "reason_codes",
                    ],
                ),
                (
                    "RunTrustSweepResponse",
                    &["request_id", "evaluated", "updated"],
                ),
                (
                    "ProviderTrustStateRecord",
                    &[
                        "provider_id",
                        "device_id",
                        "status",
                        "policy_version",
                        "trust_score",
                        "risk_score",
                        "evidence_count",
                        "successful_challenge_count",
                        "failed_challenge_count",
                        "reason_codes",
                        "created_at",
                        "updated_at",
                    ],
                ),
                ("ListProviderTrustStatesResponse", &["request_id", "states"]),
                (
                    "AntifraudEventRecord",
                    &[
                        "event_id",
                        "provider_id",
                        "device_id",
                        "event_type",
                        "severity",
                        "status",
                        "reason",
                        "metadata",
                        "first_seen_at",
                        "last_seen_at",
                        "occurrence_count",
                    ],
                ),
                ("ListAntifraudEventsResponse", &["request_id", "events"]),
                ("BenchmarkProfileThresholds", &[]),
                (
                    "UpsertBenchmarkProfileRequest",
                    &[
                        "profile_id",
                        "profile_version",
                        "workload_type",
                        "display_name",
                        "image_digest",
                        "required_backend",
                        "min_vram_gb",
                        "warmup_seconds",
                        "duration_seconds",
                        "sample_count",
                    ],
                ),
                (
                    "BenchmarkProfileRecord",
                    &[
                        "profile_id",
                        "profile_version",
                        "schema_version",
                        "workload_type",
                        "display_name",
                        "image_digest",
                        "required_backend",
                        "min_vram_gb",
                        "parameters",
                        "warmup_seconds",
                        "duration_seconds",
                        "sample_count",
                        "thresholds",
                        "status",
                        "created_at",
                        "updated_at",
                    ],
                ),
                ("UpsertBenchmarkProfileResponse", &["request_id", "profile"]),
                ("ListBenchmarkProfilesResponse", &["request_id", "profiles"]),
                ("BenchmarkResultMetrics", &[]),
                (
                    "BenchmarkResultPayload",
                    &[
                        "schema_version",
                        "provider_id",
                        "device_id",
                        "session_id",
                        "run_id",
                        "profile_id",
                        "profile_version",
                        "workload_type",
                        "backend",
                        "hardware_fingerprint",
                        "gpu_uuid",
                        "image_digest",
                        "parameters",
                        "warmup_seconds",
                        "duration_seconds",
                        "sample_count",
                        "started_at",
                        "completed_at",
                        "driver_version",
                        "metrics",
                        "warnings",
                    ],
                ),
                (
                    "SignedBenchmarkResult",
                    &[
                        "payload",
                        "result_hash",
                        "public_key_id",
                        "signature",
                        "canonicalization_version",
                    ],
                ),
                (
                    "BenchmarkResultVerification",
                    &[
                        "schema_version",
                        "result_hash_valid",
                        "signature_valid",
                        "session_bound",
                        "profile_bound",
                        "backend_bound",
                        "fingerprint_bound",
                        "image_bound",
                        "model_bound",
                        "artifact_bound",
                        "profile_configuration_bound",
                        "metrics_satisfied",
                        "warnings",
                        "errors",
                    ],
                ),
                (
                    "BenchmarkResultRecord",
                    &[
                        "result_id",
                        "provider_id",
                        "device_id",
                        "session_id",
                        "run_id",
                        "profile_id",
                        "profile_version",
                        "schema_version",
                        "workload_type",
                        "backend",
                        "hardware_fingerprint",
                        "gpu_uuid",
                        "image_digest",
                        "parameters",
                        "warmup_seconds",
                        "duration_seconds",
                        "sample_count",
                        "started_at",
                        "completed_at",
                        "server_received_at",
                        "driver_version",
                        "metrics",
                        "result_hash",
                        "public_key_id",
                        "status",
                        "verification",
                        "warnings",
                    ],
                ),
                (
                    "SubmitBenchmarkResultResponse",
                    &["request_id", "duplicate", "result"],
                ),
                (
                    "ListProviderBenchmarkResultsResponse",
                    &["request_id", "results"],
                ),
                ("WorkloadPolicyRequirements", &[]),
                (
                    "UpsertWorkloadPolicyRequest",
                    &[
                        "policy_id",
                        "policy_version",
                        "workload_type",
                        "display_name",
                    ],
                ),
                (
                    "WorkloadPolicyRecord",
                    &[
                        "policy_id",
                        "policy_version",
                        "schema_version",
                        "workload_type",
                        "display_name",
                        "requirements",
                        "status",
                        "created_at",
                        "updated_at",
                    ],
                ),
                ("UpsertWorkloadPolicyResponse", &["request_id", "policy"]),
                ("ListWorkloadPoliciesResponse", &["request_id", "policies"]),
                ("RunWorkloadEligibilityRequest", &[]),
                (
                    "WorkloadEligibilityRecord",
                    &[
                        "provider_id",
                        "device_id",
                        "workload_type",
                        "policy_id",
                        "policy_version",
                        "schema_version",
                        "engine_version",
                        "status",
                        "reason_codes",
                        "evaluated_at",
                        "updated_at",
                    ],
                ),
                (
                    "RunWorkloadEligibilityResponse",
                    &["request_id", "evaluated", "updated"],
                ),
                (
                    "ListProviderWorkloadEligibilityResponse",
                    &["request_id", "states"],
                ),
            ],
        );
    }

    for (path, method, schema) in [
        ("/v1/providers", "post", "CreateProviderRequest"),
        ("/v1/enrollments", "post", "StartEnrollmentRequest"),
        (
            "/v1/enrollments/{enrollment_id}/proof",
            "post",
            "EnrollmentProofRequest",
        ),
        (
            "/v1/devices/{device_id}/key-rotations",
            "post",
            "StartKeyRotationRequest",
        ),
        (
            "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof",
            "post",
            "KeyRotationProofRequest",
        ),
        ("/v1/sessions", "post", "StartRemoteSessionRequest"),
        (
            "/v1/sessions/{session_id}/heartbeats",
            "post",
            "HeartbeatControlMessage",
        ),
        (
            "/v1/sessions/{session_id}/telemetry-batches",
            "post",
            "TelemetryBatchControlMessage",
        ),
        (
            "/v1/sessions/{session_id}/evidence-records",
            "post",
            "SubmitEvidenceRequest",
        ),
        (
            "/v1/evidence-records/{evidence_id}/revoke",
            "post",
            "RevokeEvidenceRequest",
        ),
        ("/v1/challenges", "post", "IssueProofChallengeRequest"),
        (
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
            "post",
            "SignedProofCapabilityResponse",
        ),
        (
            "/v1/verification/sweep",
            "post",
            "RunVerificationSweepRequest",
        ),
        (
            "/v1/network-probes/observations",
            "post",
            "SubmitNetworkProbeObservationRequest",
        ),
        ("/v1/trust/sweep", "post", "RunTrustSweepRequest"),
        (
            "/v1/benchmark-profiles",
            "post",
            "UpsertBenchmarkProfileRequest",
        ),
        (
            "/v1/sessions/{session_id}/benchmark-results",
            "post",
            "SignedBenchmarkResult",
        ),
        (
            "/v1/workload-policies",
            "post",
            "UpsertWorkloadPolicyRequest",
        ),
        (
            "/v1/workload-eligibility/sweep",
            "post",
            "RunWorkloadEligibilityRequest",
        ),
    ] {
        set_request_body(document, path, method, schema);
    }

    for (path, method, status, schema) in [
        ("/health", "get", "200", "HealthResponse"),
        ("/ready", "get", "200", "ReadyResponse"),
        ("/v1/providers", "post", "201", "ProviderEnvelope"),
        (
            "/v1/providers/{provider_id}",
            "get",
            "200",
            "ProviderEnvelope",
        ),
        (
            "/v1/providers/{provider_id}/enrollment-tokens",
            "post",
            "201",
            "IssueEnrollmentTokenResponse",
        ),
        ("/v1/enrollments", "post", "202", "StartEnrollmentResponse"),
        (
            "/v1/enrollments/{enrollment_id}/proof",
            "post",
            "201",
            "EnrollmentProofResponse",
        ),
        (
            "/v1/providers/{provider_id}/devices",
            "get",
            "200",
            "ListProviderDevicesResponse",
        ),
        (
            "/v1/devices/{device_id}/credentials",
            "post",
            "201",
            "DeviceCredentialResponse",
        ),
        (
            "/v1/devices/{device_id}/key-rotations",
            "post",
            "202",
            "StartKeyRotationResponse",
        ),
        (
            "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof",
            "post",
            "200",
            "KeyRotationProofResponse",
        ),
        (
            "/v1/devices/{device_id}/revoke",
            "post",
            "200",
            "DeviceRevocationResponse",
        ),
        ("/v1/sessions", "post", "201", "StartRemoteSessionResponse"),
        (
            "/v1/sessions/{session_id}",
            "get",
            "200",
            "RemoteSessionRecord",
        ),
        (
            "/v1/sessions/{session_id}/heartbeats",
            "post",
            "200",
            "HeartbeatReceipt",
        ),
        (
            "/v1/sessions/{session_id}/revoke",
            "post",
            "200",
            "RemoteSessionRevocationResponse",
        ),
        (
            "/v1/sessions/{session_id}/telemetry-batches",
            "post",
            "200",
            "TelemetryBatchReceipt",
        ),
        (
            "/v1/sessions/{session_id}/telemetry/latest",
            "get",
            "200",
            "LatestTelemetryResponse",
        ),
        (
            "/v1/sessions/{session_id}/evidence-records",
            "post",
            "201",
            "SubmitEvidenceResponse",
        ),
        (
            "/v1/sessions/{session_id}/evidence-records",
            "post",
            "200",
            "SubmitEvidenceResponse",
        ),
        (
            "/v1/providers/{provider_id}/evidence-records",
            "get",
            "200",
            "ListEvidenceResponse",
        ),
        (
            "/v1/evidence-records/{evidence_id}",
            "get",
            "200",
            "EvidenceRecord",
        ),
        (
            "/v1/evidence-records/{evidence_id}/revoke",
            "post",
            "200",
            "RevokeEvidenceResponse",
        ),
        (
            "/v1/challenges",
            "post",
            "201",
            "IssueProofChallengeResponse",
        ),
        (
            "/v1/challenges/{challenge_id}",
            "get",
            "200",
            "ProofChallengeRecord",
        ),
        (
            "/v1/sessions/{session_id}/challenges/next",
            "get",
            "200",
            "NextProofChallengeResponse",
        ),
        (
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
            "post",
            "200",
            "SubmitProofChallengeResponse",
        ),
        (
            "/v1/verification/sweep",
            "post",
            "202",
            "RunVerificationSweepResponse",
        ),
        (
            "/v1/providers/{provider_id}/verification-states",
            "get",
            "200",
            "ListVerificationStatesResponse",
        ),
        (
            "/v1/network-probes/observations",
            "post",
            "201",
            "SubmitNetworkProbeObservationResponse",
        ),
        (
            "/v1/network-probes/observations",
            "post",
            "200",
            "SubmitNetworkProbeObservationResponse",
        ),
        (
            "/v1/providers/{provider_id}/network-probes",
            "get",
            "200",
            "ListNetworkProbeObservationsResponse",
        ),
        (
            "/v1/providers/{provider_id}/network-state",
            "get",
            "200",
            "ListProviderNetworkStatesResponse",
        ),
        ("/v1/trust/sweep", "post", "202", "RunTrustSweepResponse"),
        (
            "/v1/providers/{provider_id}/trust-states",
            "get",
            "200",
            "ListProviderTrustStatesResponse",
        ),
        (
            "/v1/providers/{provider_id}/antifraud-events",
            "get",
            "200",
            "ListAntifraudEventsResponse",
        ),
        (
            "/v1/benchmark-profiles",
            "post",
            "201",
            "UpsertBenchmarkProfileResponse",
        ),
        (
            "/v1/benchmark-profiles",
            "get",
            "200",
            "ListBenchmarkProfilesResponse",
        ),
        (
            "/v1/sessions/{session_id}/benchmark-results",
            "post",
            "201",
            "SubmitBenchmarkResultResponse",
        ),
        (
            "/v1/sessions/{session_id}/benchmark-results",
            "post",
            "200",
            "SubmitBenchmarkResultResponse",
        ),
        (
            "/v1/providers/{provider_id}/benchmark-results",
            "get",
            "200",
            "ListProviderBenchmarkResultsResponse",
        ),
        (
            "/v1/workload-policies",
            "post",
            "201",
            "UpsertWorkloadPolicyResponse",
        ),
        (
            "/v1/workload-policies",
            "get",
            "200",
            "ListWorkloadPoliciesResponse",
        ),
        (
            "/v1/workload-eligibility/sweep",
            "post",
            "202",
            "RunWorkloadEligibilityResponse",
        ),
        (
            "/v1/providers/{provider_id}/workload-eligibility",
            "get",
            "200",
            "ListProviderWorkloadEligibilityResponse",
        ),
    ] {
        set_json_response(document, path, method, status, schema);
    }
}

fn add_control_plane_protocol_examples(document: &mut serde_json::Value) {
    {
        let examples = document["components"]["examples"]
            .as_object_mut()
            .expect("OpenAPI examples object");
        for (name, raw) in [
            (
                "StartEnrollmentRequestExample",
                START_ENROLLMENT_REQUEST_EXAMPLE_JSON,
            ),
            (
                "StartEnrollmentResponseExample",
                START_ENROLLMENT_RESPONSE_EXAMPLE_JSON,
            ),
            (
                "EnrollmentProofRequestExample",
                ENROLLMENT_PROOF_REQUEST_EXAMPLE_JSON,
            ),
            (
                "EnrollmentProofResponseExample",
                ENROLLMENT_PROOF_RESPONSE_EXAMPLE_JSON,
            ),
            (
                "StartRemoteSessionRequestExample",
                START_REMOTE_SESSION_REQUEST_EXAMPLE_JSON,
            ),
            (
                "StartRemoteSessionResponseExample",
                START_REMOTE_SESSION_RESPONSE_EXAMPLE_JSON,
            ),
            (
                "HeartbeatControlMessageExample",
                HEARTBEAT_CONTROL_MESSAGE_EXAMPLE_JSON,
            ),
            ("HeartbeatReceiptExample", HEARTBEAT_RECEIPT_EXAMPLE_JSON),
            (
                "SubmitEvidenceRequestExample",
                SUBMIT_EVIDENCE_REQUEST_EXAMPLE_JSON,
            ),
            (
                "SubmitEvidenceResponseExample",
                SUBMIT_EVIDENCE_RESPONSE_EXAMPLE_JSON,
            ),
            (
                "IssueProofChallengeRequestExample",
                ISSUE_PROOF_CHALLENGE_REQUEST_EXAMPLE_JSON,
            ),
            (
                "IssueProofChallengeResponseExample",
                ISSUE_PROOF_CHALLENGE_RESPONSE_EXAMPLE_JSON,
            ),
            (
                "SignedProofCapabilityResponseExample",
                SIGNED_PROOF_CAPABILITY_RESPONSE_EXAMPLE_JSON,
            ),
            (
                "SubmitProofChallengeResponseExample",
                SUBMIT_PROOF_CHALLENGE_RESPONSE_EXAMPLE_JSON,
            ),
        ] {
            examples.insert(
                name.to_string(),
                serde_json::json!({ "value": control_plane_protocol_example_value(name, raw) }),
            );
        }
    }

    for (path, method, example_key, component_example) in [
        (
            "/v1/enrollments",
            "post",
            "start_enrollment",
            "StartEnrollmentRequestExample",
        ),
        (
            "/v1/enrollments/{enrollment_id}/proof",
            "post",
            "enrollment_proof",
            "EnrollmentProofRequestExample",
        ),
        (
            "/v1/sessions",
            "post",
            "start_remote_session",
            "StartRemoteSessionRequestExample",
        ),
        (
            "/v1/sessions/{session_id}/heartbeats",
            "post",
            "heartbeat",
            "HeartbeatControlMessageExample",
        ),
        (
            "/v1/sessions/{session_id}/evidence-records",
            "post",
            "submit_evidence",
            "SubmitEvidenceRequestExample",
        ),
        (
            "/v1/challenges",
            "post",
            "issue_proof_challenge",
            "IssueProofChallengeRequestExample",
        ),
        (
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
            "post",
            "signed_proof_response",
            "SignedProofCapabilityResponseExample",
        ),
    ] {
        set_json_request_example(document, path, method, example_key, component_example);
    }

    for (path, method, status, example_key, component_example) in [
        (
            "/v1/enrollments",
            "post",
            "202",
            "start_enrollment",
            "StartEnrollmentResponseExample",
        ),
        (
            "/v1/enrollments/{enrollment_id}/proof",
            "post",
            "201",
            "enrollment_proof",
            "EnrollmentProofResponseExample",
        ),
        (
            "/v1/sessions",
            "post",
            "201",
            "start_remote_session",
            "StartRemoteSessionResponseExample",
        ),
        (
            "/v1/sessions/{session_id}/heartbeats",
            "post",
            "200",
            "heartbeat",
            "HeartbeatReceiptExample",
        ),
        (
            "/v1/sessions/{session_id}/evidence-records",
            "post",
            "201",
            "submit_evidence",
            "SubmitEvidenceResponseExample",
        ),
        (
            "/v1/sessions/{session_id}/evidence-records",
            "post",
            "200",
            "submit_evidence_duplicate",
            "SubmitEvidenceResponseExample",
        ),
        (
            "/v1/challenges",
            "post",
            "201",
            "issue_proof_challenge",
            "IssueProofChallengeResponseExample",
        ),
        (
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
            "post",
            "200",
            "signed_proof_response",
            "SubmitProofChallengeResponseExample",
        ),
    ] {
        set_json_response_example(
            document,
            path,
            method,
            status,
            example_key,
            component_example,
        );
    }
}

fn control_plane_protocol_example_value(name: &str, raw: &str) -> serde_json::Value {
    serde_json::from_str(raw)
        .unwrap_or_else(|error| panic!("invalid OpenAPI example fixture {name}: {error}"))
}
fn insert_structural_schemas(
    schemas: &mut serde_json::Map<String, serde_json::Value>,
    definitions: &[(&str, &[&str])],
) {
    for (name, required) in definitions {
        insert_schema(schemas, name, structural_object_schema(name, required));
    }
}

fn structural_object_schema(contract: &str, required: &[&str]) -> serde_json::Value {
    let properties = required
        .iter()
        .map(|field| ((*field).to_string(), serde_json::json!({})))
        .collect::<serde_json::Map<_, _>>();
    serde_json::json!({
        "type": "object",
        "required": required,
        "properties": properties,
        "additionalProperties": true,
        "description": format!("Structural OpenAPI contract for {contract}; nested and optional fields follow the Rust serde contract in burd-protocol/control-plane.")
    })
}

fn insert_schema(
    schemas: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    schema: serde_json::Value,
) {
    schemas.insert(name.to_string(), schema);
}

fn add_jobs_scheduler_reservation_contracts(document: &mut serde_json::Value) {
    let schemas = document["components"]["schemas"]
        .as_object_mut()
        .expect("OpenAPI schemas object");
    schemas.insert(
        "JobArtifact".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["artifact_id", "role", "object_key"],
            "properties": {
                "artifact_id": { "type": "string" },
                "role": { "type": "string", "enum": ["input", "output", "model", "config", "log", "artifact"] },
                "object_key": { "type": "string" },
                "sha256": { "type": ["string", "null"] },
                "size_bytes": { "type": ["integer", "null"], "minimum": 0 },
                "content_type": { "type": ["string", "null"] }
            }
        }),
    );
    schemas.insert(
        "CreateJobRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["provider_id", "device_id", "session_id", "workload_type", "template_id", "image_ref", "gpu_uuid", "backend"],
            "properties": {
                "client_job_id": { "type": ["string", "null"] },
                "provider_id": { "type": "string" },
                "device_id": { "type": "string" },
                "session_id": { "type": "string" },
                "workload_type": { "type": "string" },
                "template_id": { "type": "string", "enum": ["llm_inference", "embeddings", "image_generation", "transcription"] },
                "image_ref": { "type": "string", "description": "Digest-pinned image reference; tag-only references are rejected by runtime validation." },
                "gpu_uuid": { "type": "string" },
                "backend": { "type": "string", "enum": ["cuda"] },
                "parameters": { "type": "object", "additionalProperties": true },
                "input_artifacts": { "type": "array", "items": { "$ref": "#/components/schemas/JobArtifact" } },
                "expected_outputs": { "type": "array", "items": { "$ref": "#/components/schemas/JobArtifact" } },
                "timeout_seconds": { "type": ["integer", "null"], "minimum": 1, "maximum": 86400 },
                "policy_id": { "type": ["string", "null"] },
                "policy_version": { "type": ["string", "null"] }
            }
        }),
    );
    schemas.insert(
        "JobRecord".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["job_id", "provider_id", "device_id", "session_id", "schema_version", "workload_type", "template_id", "image_ref", "gpu_uuid", "backend", "status", "timeout_seconds", "created_at", "updated_at"],
            "properties": {
                "job_id": { "type": "string" },
                "client_job_id": { "type": ["string", "null"] },
                "provider_id": { "type": "string" },
                "device_id": { "type": "string" },
                "session_id": { "type": "string" },
                "schema_version": { "type": "string", "const": "burd-job-v1" },
                "workload_type": { "type": "string" },
                "template_id": { "type": "string" },
                "image_ref": { "type": "string" },
                "gpu_uuid": { "type": "string" },
                "backend": { "type": "string", "enum": ["cuda"] },
                "parameters": { "type": "object", "additionalProperties": true },
                "input_artifacts": { "type": "array", "items": { "$ref": "#/components/schemas/JobArtifact" } },
                "expected_outputs": { "type": "array", "items": { "$ref": "#/components/schemas/JobArtifact" } },
                "result_artifacts": { "type": "array", "items": { "$ref": "#/components/schemas/JobArtifact" } },
                "policy_id": { "type": ["string", "null"] },
                "policy_version": { "type": ["string", "null"] },
                "status": { "type": "string", "enum": ["queued", "assigned", "accepted", "provisioning", "running", "uploading", "succeeded", "failed", "cancelled"] },
                "progress_percent": { "type": ["number", "null"], "minimum": 0, "maximum": 100 },
                "status_message": { "type": ["string", "null"] },
                "error_code": { "type": ["string", "null"] },
                "error_message": { "type": ["string", "null"] },
                "cancellation_reason": { "type": ["string", "null"] },
                "timeout_seconds": { "type": "integer", "minimum": 1 },
                "created_at": { "type": "string", "format": "date-time" },
                "assigned_at": { "type": ["string", "null"], "format": "date-time" },
                "accepted_at": { "type": ["string", "null"], "format": "date-time" },
                "started_at": { "type": ["string", "null"], "format": "date-time" },
                "completed_at": { "type": ["string", "null"], "format": "date-time" },
                "updated_at": { "type": "string", "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "CreateJobResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "job", "duplicate"],
            "properties": {
                "request_id": { "type": "string" },
                "job": { "$ref": "#/components/schemas/JobRecord" },
                "duplicate": { "type": "boolean" }
            }
        }),
    );
    schemas.insert(
        "JobResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "job"],
            "properties": {
                "request_id": { "type": "string" },
                "job": { "$ref": "#/components/schemas/JobRecord" }
            }
        }),
    );
    schemas.insert(
        "ListJobsResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "jobs"],
            "properties": {
                "request_id": { "type": "string" },
                "jobs": { "type": "array", "items": { "$ref": "#/components/schemas/JobRecord" } }
            }
        }),
    );
    schemas.insert(
        "JobDataPlaneUrl".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["artifact_id", "method", "url", "expires_at"],
            "properties": {
                "artifact_id": { "type": "string" },
                "method": { "type": "string", "enum": ["GET", "PUT"] },
                "url": { "type": "string" },
                "expires_at": { "type": "string", "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "JobDataPlaneGrant".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["schema_version", "job_id", "credential", "credential_expires_at", "download_urls", "upload_urls"],
            "properties": {
                "schema_version": { "type": "string", "const": "burd-job-data-plane-grant-v1" },
                "job_id": { "type": "string" },
                "credential": { "type": "string", "description": "Returned once to the authorized provider session; clients must redact it from logs." },
                "credential_expires_at": { "type": "string", "format": "date-time" },
                "download_urls": { "type": "array", "items": { "$ref": "#/components/schemas/JobDataPlaneUrl" } },
                "upload_urls": { "type": "array", "items": { "$ref": "#/components/schemas/JobDataPlaneUrl" } }
            }
        }),
    );
    schemas.insert(
        "ProviderJobExecutionState".to_string(),
        serde_json::json!({
            "type": "string",
            "enum": ["assigned", "accepted", "provisioning", "running", "uploading", "succeeded", "failed", "cancelled", "expired"]
        }),
    );
    schemas.insert(
        "ProviderJobRuntimePolicy".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["runtime_engine", "target_os", "command_source", "command_override_allowed", "entrypoint_override_allowed", "network_mode", "read_only_rootfs", "no_new_privileges", "run_as_user", "seccomp_profile", "cap_drop", "cpu_millis", "memory_mib", "pids_limit", "shm_size_mib"],
            "properties": {
                "runtime_engine": { "type": "string", "const": "docker" },
                "target_os": { "type": "string", "const": "linux" },
                "command_source": { "type": "string", "const": "approved_template" },
                "command_override_allowed": { "type": "boolean", "const": false },
                "entrypoint_override_allowed": { "type": "boolean", "const": false },
                "network_mode": { "type": "string", "const": "none" },
                "read_only_rootfs": { "type": "boolean", "const": true },
                "no_new_privileges": { "type": "boolean", "const": true },
                "run_as_user": { "type": "string", "const": "1000:1000" },
                "seccomp_profile": { "type": "string", "const": "default" },
                "cap_drop": { "type": "array", "items": { "type": "string" } },
                "cpu_millis": { "type": "integer", "minimum": 1 },
                "memory_mib": { "type": "integer", "minimum": 1 },
                "pids_limit": { "type": "integer", "minimum": 1 },
                "shm_size_mib": { "type": "integer", "minimum": 1 }
            }
        }),
    );
    schemas.insert(
        "ProviderJobCancellationPolicy".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["poll_interval_seconds", "graceful_stop_seconds", "force_kill_after_seconds"],
            "properties": {
                "poll_interval_seconds": { "type": "integer", "minimum": 1 },
                "graceful_stop_seconds": { "type": "integer", "minimum": 1 },
                "force_kill_after_seconds": { "type": "integer", "minimum": 1 }
            }
        }),
    );
    schemas.insert(
        "ProviderJobCleanupPolicy".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["remove_container", "remove_working_directory", "clear_ephemeral_secrets", "revoke_data_plane_credential"],
            "properties": {
                "remove_container": { "type": "boolean", "const": true },
                "remove_working_directory": { "type": "boolean", "const": true },
                "clear_ephemeral_secrets": { "type": "boolean", "const": true },
                "revoke_data_plane_credential": { "type": "boolean", "const": true }
            }
        }),
    );
    schemas.insert(
        "ProviderJobExecutionSpec".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["schema_version", "policy_version", "job_schema_version", "lease_schema_version", "data_plane_schema_version", "job_id", "lease_id", "provider_id", "device_id", "session_id", "workload_type", "template_id", "image_ref", "gpu_uuid", "backend", "initial_state", "timeout_seconds", "lease_expires_at", "data_plane_credential_expires_at", "runtime", "cancellation", "cleanup"],
            "properties": {
                "schema_version": { "type": "string", "const": "burd-provider-job-execution-v1" },
                "policy_version": { "type": "string", "const": "burd-provider-job-runtime-policy-v1" },
                "job_schema_version": { "type": "string" },
                "lease_schema_version": { "type": "string" },
                "data_plane_schema_version": { "type": "string" },
                "job_id": { "type": "string" },
                "lease_id": { "type": "string" },
                "provider_id": { "type": "string" },
                "device_id": { "type": "string" },
                "session_id": { "type": "string" },
                "workload_type": { "type": "string" },
                "template_id": { "type": "string", "enum": ["llm_inference", "embeddings", "image_generation", "whisper_transcription", "file_processing"] },
                "image_ref": { "type": "string", "description": "Digest-pinned container image selected by the backend." },
                "gpu_uuid": { "type": "string" },
                "backend": { "type": "string", "const": "cuda" },
                "policy_id": { "type": ["string", "null"] },
                "workload_policy_version": { "type": ["string", "null"] },
                "initial_state": { "$ref": "#/components/schemas/ProviderJobExecutionState" },
                "timeout_seconds": { "type": "integer", "minimum": 1 },
                "lease_expires_at": { "type": "string", "format": "date-time" },
                "data_plane_credential_expires_at": { "type": "string", "format": "date-time" },
                "runtime": { "$ref": "#/components/schemas/ProviderJobRuntimePolicy" },
                "cancellation": { "$ref": "#/components/schemas/ProviderJobCancellationPolicy" },
                "cleanup": { "$ref": "#/components/schemas/ProviderJobCleanupPolicy" }
            }
        }),
    );
    schemas.insert(
        "NextJobResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id"],
            "properties": {
                "request_id": { "type": "string" },
                "job": { "oneOf": [{ "$ref": "#/components/schemas/JobRecord" }, { "type": "null" }] },
                "data_plane": { "oneOf": [{ "$ref": "#/components/schemas/JobDataPlaneGrant" }, { "type": "null" }] },
                "lease": { "oneOf": [{ "$ref": "#/components/schemas/JobLeaseRecord" }, { "type": "null" }] },
                "execution": { "oneOf": [{ "$ref": "#/components/schemas/ProviderJobExecutionSpec" }, { "type": "null" }] }
            }
        }),
    );
    schemas.insert(
        "AcceptJobRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": { "status_message": { "type": ["string", "null"] } }
        }),
    );
    schemas.insert(
        "JobEventRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["sequence", "event_type"],
            "properties": {
                "sequence": { "type": "integer", "minimum": 1 },
                "event_type": { "type": "string", "enum": ["provisioning", "started", "running", "uploading", "progress"] },
                "progress_percent": { "type": ["number", "null"], "minimum": 0, "maximum": 100 },
                "message": { "type": ["string", "null"] },
                "metadata": { "type": "object", "additionalProperties": true },
                "occurred_at": { "type": ["string", "null"], "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "JobEventRecord".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["event_id", "job_id", "provider_id", "device_id", "session_id", "sequence", "schema_version", "event_type", "metadata", "occurred_at", "server_received_at"],
            "properties": {
                "event_id": { "type": "string" },
                "job_id": { "type": "string" },
                "provider_id": { "type": "string" },
                "device_id": { "type": "string" },
                "session_id": { "type": "string" },
                "sequence": { "type": "integer", "minimum": 1 },
                "schema_version": { "type": "string", "const": "burd-job-event-v1" },
                "event_type": { "type": "string" },
                "progress_percent": { "type": ["number", "null"], "minimum": 0, "maximum": 100 },
                "message": { "type": ["string", "null"] },
                "metadata": { "type": "object", "additionalProperties": true },
                "occurred_at": { "type": "string", "format": "date-time" },
                "server_received_at": { "type": "string", "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "JobEventResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "event", "job"],
            "properties": {
                "request_id": { "type": "string" },
                "event": { "$ref": "#/components/schemas/JobEventRecord" },
                "job": { "$ref": "#/components/schemas/JobRecord" }
            }
        }),
    );
    schemas.insert(
        "SubmitJobResultRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["status"],
            "properties": {
                "status": { "type": "string", "enum": ["succeeded", "failed"] },
                "result_artifacts": { "type": "array", "items": { "$ref": "#/components/schemas/JobArtifact" } },
                "metrics": { "type": "object", "additionalProperties": true },
                "error_code": { "type": ["string", "null"] },
                "error_message": { "type": ["string", "null"] },
                "completed_at": { "type": ["string", "null"], "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "SubmitJobResultResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "job"],
            "properties": {
                "request_id": { "type": "string" },
                "job": { "$ref": "#/components/schemas/JobRecord" }
            }
        }),
    );
    schemas.insert(
        "CancelJobRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": { "reason": { "type": ["string", "null"] } }
        }),
    );
    schemas.insert(
        "JobLeaseRecord".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["lease_id", "job_id", "provider_id", "device_id", "session_id", "schema_version", "workload_type", "gpu_uuid", "status", "reason_codes", "offered_at", "expires_at", "created_at", "updated_at"],
            "properties": {
                "lease_id": { "type": "string" },
                "job_id": { "type": "string" },
                "provider_id": { "type": "string" },
                "device_id": { "type": "string" },
                "session_id": { "type": "string" },
                "schema_version": { "type": "string", "const": "burd-job-lease-v1" },
                "workload_type": { "type": "string" },
                "gpu_uuid": { "type": "string" },
                "policy_id": { "type": ["string", "null"] },
                "policy_version": { "type": ["string", "null"] },
                "status": { "type": "string", "enum": ["offered", "accepted", "provisioning", "active", "completed", "failed", "expired"] },
                "reason_codes": { "type": "array", "items": { "type": "string" } },
                "offered_at": { "type": "string", "format": "date-time" },
                "expires_at": { "type": "string", "format": "date-time" },
                "accepted_at": { "type": ["string", "null"], "format": "date-time" },
                "provisioning_at": { "type": ["string", "null"], "format": "date-time" },
                "active_at": { "type": ["string", "null"], "format": "date-time" },
                "completed_at": { "type": ["string", "null"], "format": "date-time" },
                "failure_reason": { "type": ["string", "null"] },
                "created_at": { "type": "string", "format": "date-time" },
                "updated_at": { "type": "string", "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "RunSchedulerRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": 200 },
                "lease_ttl_seconds": { "type": ["integer", "null"], "minimum": 1, "maximum": 900 },
                "reason": { "type": ["string", "null"] }
            }
        }),
    );
    schemas.insert(
        "SchedulerDecisionRecord".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["job_id", "provider_id", "device_id", "session_id", "gpu_uuid", "decision", "reason_codes"],
            "properties": {
                "job_id": { "type": "string" },
                "lease_id": { "type": ["string", "null"] },
                "provider_id": { "type": "string" },
                "device_id": { "type": "string" },
                "session_id": { "type": "string" },
                "gpu_uuid": { "type": "string" },
                "decision": { "type": "string", "enum": ["offered", "skipped", "expired"] },
                "reason_codes": { "type": "array", "items": { "type": "string" } }
            }
        }),
    );
    schemas.insert(
        "RunSchedulerResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "evaluated", "offered", "expired", "skipped", "decisions"],
            "properties": {
                "request_id": { "type": "string" },
                "evaluated": { "type": "integer", "minimum": 0 },
                "offered": { "type": "integer", "minimum": 0 },
                "expired": { "type": "integer", "minimum": 0 },
                "skipped": { "type": "integer", "minimum": 0 },
                "decisions": { "type": "array", "items": { "$ref": "#/components/schemas/SchedulerDecisionRecord" } }
            }
        }),
    );
    schemas.insert(
        "ListJobLeasesResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "leases"],
            "properties": {
                "request_id": { "type": "string" },
                "leases": { "type": "array", "items": { "$ref": "#/components/schemas/JobLeaseRecord" } }
            }
        }),
    );
    schemas.insert(
        "CreateReservationRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["listing_id", "duration_seconds"],
            "properties": {
                "listing_id": { "type": "string" },
                "duration_seconds": { "type": "integer", "minimum": 1, "maximum": 86400 },
                "starts_at": { "type": ["string", "null"], "format": "date-time" },
                "workload_type": { "type": ["string", "null"] }
            }
        }),
    );
    schemas.insert(
        "CancelReservationRequest".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": { "reason": { "type": ["string", "null"] } }
        }),
    );
    schemas.insert(
        "MarketplaceReservationRecord".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["reservation_id", "organization_id", "project_id", "listing_id", "provider_id", "device_id", "schema_version", "workload_type", "status", "starts_at", "expires_at", "reserved_gpu_seconds", "reason_codes", "created_at", "updated_at"],
            "properties": {
                "reservation_id": { "type": "string" },
                "organization_id": { "type": "string" },
                "project_id": { "type": "string" },
                "listing_id": { "type": "string" },
                "provider_id": { "type": "string" },
                "device_id": { "type": "string" },
                "session_id": { "type": ["string", "null"] },
                "schema_version": { "type": "string", "const": "burd-marketplace-reservation-v1" },
                "workload_type": { "type": "string" },
                "gpu_uuid": { "type": ["string", "null"] },
                "status": { "type": "string", "enum": ["reserved", "cancelled", "expired"] },
                "starts_at": { "type": "string", "format": "date-time" },
                "expires_at": { "type": "string", "format": "date-time" },
                "cancelled_at": { "type": ["string", "null"], "format": "date-time" },
                "reserved_gpu_seconds": { "type": "integer", "minimum": 1 },
                "reason_codes": { "type": "array", "items": { "type": "string" } },
                "created_at": { "type": "string", "format": "date-time" },
                "updated_at": { "type": "string", "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "MarketplaceReservationResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "reservation", "duplicate"],
            "properties": {
                "request_id": { "type": "string" },
                "reservation": { "$ref": "#/components/schemas/MarketplaceReservationRecord" },
                "duplicate": { "type": "boolean" }
            }
        }),
    );
    schemas.insert(
        "ListMarketplaceReservationsResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "reservations"],
            "properties": {
                "request_id": { "type": "string" },
                "reservations": { "type": "array", "items": { "$ref": "#/components/schemas/MarketplaceReservationRecord" } }
            }
        }),
    );
    schemas.insert(
        "ProviderPayoutRecord".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["payout_id", "provider_id", "payout_account_id", "schema_version", "status", "amount_micros", "currency", "created_at", "updated_at"],
            "properties": {
                "payout_id": { "type": "string" },
                "provider_id": { "type": "string" },
                "payout_account_id": { "type": "string" },
                "schema_version": { "type": "string", "const": "burd-provider-payout-v1" },
                "status": { "type": "string", "enum": ["held", "approved", "paid", "failed", "cancelled"], "description": "Current API creates held or approved payouts only. paid, failed, and cancelled are reserved for future adapter/admin transition APIs." },
                "amount_micros": { "type": "integer", "minimum": 1 },
                "currency": { "type": "string", "pattern": "^[A-Z]{3}$" },
                "hold_until": { "type": ["string", "null"], "format": "date-time" },
                "external_reference": { "type": ["string", "null"] },
                "paid_at": { "type": ["string", "null"], "format": "date-time" },
                "created_at": { "type": "string", "format": "date-time" },
                "updated_at": { "type": "string", "format": "date-time" }
            }
        }),
    );
    schemas.insert(
        "ProviderPayoutResponse".to_string(),
        serde_json::json!({
            "type": "object",
            "required": ["request_id", "payout"],
            "properties": {
                "request_id": { "type": "string" },
                "payout": { "$ref": "#/components/schemas/ProviderPayoutRecord" }
            }
        }),
    );

    set_request_body(document, "/v1/jobs", "post", "CreateJobRequest");
    set_json_response(document, "/v1/jobs", "post", "201", "CreateJobResponse");
    set_json_response(document, "/v1/jobs/{job_id}", "get", "200", "JobResponse");
    set_request_body(
        document,
        "/v1/jobs/{job_id}/cancel",
        "post",
        "CancelJobRequest",
    );
    set_json_response(
        document,
        "/v1/jobs/{job_id}/cancel",
        "post",
        "200",
        "JobResponse",
    );
    set_json_response(
        document,
        "/v1/providers/{provider_id}/jobs",
        "get",
        "200",
        "ListJobsResponse",
    );
    set_request_body(document, "/v1/scheduler/run", "post", "RunSchedulerRequest");
    set_json_response(
        document,
        "/v1/scheduler/run",
        "post",
        "202",
        "RunSchedulerResponse",
    );
    set_json_response(
        document,
        "/v1/jobs/{job_id}/leases",
        "get",
        "200",
        "ListJobLeasesResponse",
    );
    set_json_response(
        document,
        "/v1/providers/{provider_id}/leases",
        "get",
        "200",
        "ListJobLeasesResponse",
    );
    set_json_response(
        document,
        "/v1/sessions/{session_id}/jobs/next",
        "get",
        "200",
        "NextJobResponse",
    );
    set_request_body(
        document,
        "/v1/sessions/{session_id}/jobs/{job_id}/accept",
        "post",
        "AcceptJobRequest",
    );
    set_json_response(
        document,
        "/v1/sessions/{session_id}/jobs/{job_id}/accept",
        "post",
        "200",
        "JobResponse",
    );
    set_request_body(
        document,
        "/v1/sessions/{session_id}/jobs/{job_id}/events",
        "post",
        "JobEventRequest",
    );
    set_json_response(
        document,
        "/v1/sessions/{session_id}/jobs/{job_id}/events",
        "post",
        "201",
        "JobEventResponse",
    );
    set_request_body(
        document,
        "/v1/sessions/{session_id}/jobs/{job_id}/result",
        "post",
        "SubmitJobResultRequest",
    );
    set_json_response(
        document,
        "/v1/sessions/{session_id}/jobs/{job_id}/result",
        "post",
        "200",
        "SubmitJobResultResponse",
    );
    set_json_response(
        document,
        "/v1/customer/projects/{project_id}/reservations",
        "get",
        "200",
        "ListMarketplaceReservationsResponse",
    );
    set_request_body(
        document,
        "/v1/customer/projects/{project_id}/reservations",
        "post",
        "CreateReservationRequest",
    );
    set_json_response(
        document,
        "/v1/customer/projects/{project_id}/reservations",
        "post",
        "201",
        "MarketplaceReservationResponse",
    );
    set_request_body(
        document,
        "/v1/customer/reservations/{reservation_id}/cancel",
        "post",
        "CancelReservationRequest",
    );
    set_json_response(
        document,
        "/v1/customer/reservations/{reservation_id}/cancel",
        "post",
        "200",
        "MarketplaceReservationResponse",
    );
    set_json_response(
        document,
        "/v1/billing/providers/{provider_id}/payouts",
        "post",
        "201",
        "ProviderPayoutResponse",
    );
}

fn set_request_body(document: &mut serde_json::Value, path: &str, method: &str, schema: &str) {
    operation_object_mut(document, path, method).insert(
        "requestBody".to_string(),
        serde_json::json!({
            "required": true,
            "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/{schema}") } } }
        }),
    );
}

fn set_json_response(
    document: &mut serde_json::Value,
    path: &str,
    method: &str,
    status: &str,
    schema: &str,
) {
    let operation = operation_object_mut(document, path, method);
    let responses = operation
        .get_mut("responses")
        .and_then(|value| value.as_object_mut())
        .expect("OpenAPI operation responses object");
    let response = responses
        .get_mut(status)
        .and_then(|value| value.as_object_mut())
        .expect("OpenAPI response object");
    response.insert(
        "content".to_string(),
        serde_json::json!({
            "application/json": { "schema": { "$ref": format!("#/components/schemas/{schema}") } }
        }),
    );
}

fn set_json_request_example(
    document: &mut serde_json::Value,
    path: &str,
    method: &str,
    example_key: &str,
    component_example: &str,
) {
    let operation = operation_object_mut(document, path, method);
    let json_content = operation
        .get_mut("requestBody")
        .and_then(|value| value.get_mut("content"))
        .and_then(|value| value.get_mut("application/json"))
        .and_then(|value| value.as_object_mut())
        .expect("OpenAPI JSON request content object");
    let examples = json_content
        .entry("examples".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("OpenAPI JSON request examples object");
    examples.insert(
        example_key.to_string(),
        serde_json::json!({ "$ref": format!("#/components/examples/{component_example}") }),
    );
}

fn set_json_response_example(
    document: &mut serde_json::Value,
    path: &str,
    method: &str,
    status: &str,
    example_key: &str,
    component_example: &str,
) {
    let operation = operation_object_mut(document, path, method);
    let json_content = operation
        .get_mut("responses")
        .and_then(|value| value.get_mut(status))
        .and_then(|value| value.get_mut("content"))
        .and_then(|value| value.get_mut("application/json"))
        .and_then(|value| value.as_object_mut())
        .expect("OpenAPI JSON response content object");
    let examples = json_content
        .entry("examples".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("OpenAPI JSON response examples object");
    examples.insert(
        example_key.to_string(),
        serde_json::json!({ "$ref": format!("#/components/examples/{component_example}") }),
    );
}
fn operation_object_mut<'a>(
    document: &'a mut serde_json::Value,
    path: &str,
    method: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
    document
        .get_mut("paths")
        .and_then(|value| value.get_mut(path))
        .and_then(|value| value.get_mut(method))
        .and_then(|value| value.as_object_mut())
        .expect("OpenAPI operation object")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_lists_bn21_identity_session_telemetry_evidence_challenge_verification_network_trust_benchmark_workload_job_scheduler_metering_marketplace_customer_billing_observability_and_security_endpoints()
     {
        let document = document();
        let paths = document["paths"].as_object().unwrap();
        for path in [
            "/health",
            "/ready",
            "/metrics",
            "/v1/observability/snapshot",
            "/v1/security/policy",
            "/v1/providers",
            "/v1/providers/{provider_id}",
            "/v1/providers/{provider_id}/enrollment-tokens",
            "/v1/enrollments",
            "/v1/enrollments/{enrollment_id}/proof",
            "/v1/providers/{provider_id}/devices",
            "/v1/devices/{device_id}/credentials",
            "/v1/devices/{device_id}/key-rotations",
            "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof",
            "/v1/devices/{device_id}/revoke",
            "/v1/sessions",
            "/v1/sessions/{session_id}",
            "/v1/sessions/{session_id}/control",
            "/v1/sessions/{session_id}/heartbeats",
            "/v1/sessions/{session_id}/revoke",
            "/v1/sessions/{session_id}/telemetry-batches",
            "/v1/sessions/{session_id}/telemetry/latest",
            "/v1/sessions/{session_id}/evidence-records",
            "/v1/providers/{provider_id}/evidence-records",
            "/v1/providers/{provider_id}/security-postures",
            "/v1/providers/{provider_id}/gpu-inventory",
            "/v1/evidence-records/{evidence_id}",
            "/v1/evidence-records/{evidence_id}/revoke",
            "/v1/network-probes/observations",
            "/v1/providers/{provider_id}/network-probes",
            "/v1/providers/{provider_id}/network-state",
            "/v1/benchmark-profiles",
            "/v1/sessions/{session_id}/benchmark-results",
            "/v1/providers/{provider_id}/benchmark-results",
            "/v1/workload-policies",
            "/v1/workload-eligibility/sweep",
            "/v1/providers/{provider_id}/workload-eligibility",
            "/v1/marketplace/listings",
            "/v1/marketplace/listings/sweep",
            "/v1/marketplace/listings/{listing_id}/price",
            "/v1/billing/projects/{project_id}/pix/payment-intents",
            "/v1/billing/pix/payment-intents/{payment_intent_id}/confirm",
            "/v1/billing/projects/{project_id}/balance",
            "/v1/billing/projects/{project_id}/ledger",
            "/v1/billing/reservations/{reservation_id}/settle",
            "/v1/billing/invoices/{invoice_id}",
            "/v1/billing/providers/{provider_id}/balance",
            "/v1/billing/providers/{provider_id}/ledger",
            "/v1/billing/providers/{provider_id}/payout-account",
            "/v1/billing/providers/{provider_id}/payouts",
            "/v1/customer/users",
            "/v1/customer/organizations",
            "/v1/customer/organizations/{organization_id}",
            "/v1/customer/organizations/{organization_id}/projects",
            "/v1/customer/organizations/{organization_id}/audit-events",
            "/v1/customer/projects/{project_id}/quotas",
            "/v1/customer/projects/{project_id}/api-keys",
            "/v1/customer/projects/{project_id}/credits",
            "/v1/customer/projects/{project_id}/reservations",
            "/v1/customer/projects/{project_id}/usage",
            "/v1/customer/reservations/{reservation_id}/cancel",
            "/v1/jobs",
            "/v1/jobs/{job_id}",
            "/v1/jobs/{job_id}/usage-ledger",
            "/v1/jobs/{job_id}/usage-ledger/finalize",
            "/v1/jobs/{job_id}/leases",
            "/v1/jobs/{job_id}/cancel",
            "/v1/providers/{provider_id}/jobs",
            "/v1/providers/{provider_id}/marketplace-listings",
            "/v1/providers/{provider_id}/usage-ledger",
            "/v1/providers/{provider_id}/leases",
            "/v1/scheduler/run",
            "/v1/sessions/{session_id}/jobs/next",
            "/v1/sessions/{session_id}/jobs/{job_id}/accept",
            "/v1/sessions/{session_id}/jobs/{job_id}/events",
            "/v1/sessions/{session_id}/jobs/{job_id}/result",
            "/v1/trust/sweep",
            "/v1/providers/{provider_id}/trust-states",
            "/v1/providers/{provider_id}/antifraud-events",
            "/v1/verification/sweep",
            "/v1/providers/{provider_id}/verification-states",
            "/v1/challenges",
            "/v1/challenges/{challenge_id}",
            "/v1/sessions/{session_id}/security-posture",
            "/v1/sessions/{session_id}/gpu-inventory",
            "/v1/sessions/{session_id}/challenges/next",
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
        ] {
            assert!(paths.contains_key(path), "missing OpenAPI path {path}");
        }
        assert!(document["components"]["securitySchemes"]["adminBearer"].is_object());
        assert!(document["components"]["securitySchemes"]["deviceBearer"].is_object());
        assert!(document["components"]["securitySchemes"]["customerBearer"].is_object());
    }
    #[test]
    fn openapi_documents_security_boundaries_and_idempotency_header_limits() {
        let document = document();
        let paths = document["paths"].as_object().unwrap();
        for (path, method, scheme) in [
            ("/v1/providers", "post", "adminBearer"),
            ("/v1/security/policy", "get", "adminBearer"),
            ("/v1/marketplace/listings", "get", "adminBearer"),
            (
                "/v1/billing/reservations/{reservation_id}/settle",
                "post",
                "adminBearer",
            ),
            (
                "/v1/customer/projects/{project_id}/reservations",
                "post",
                "customerBearer",
            ),
            (
                "/v1/billing/projects/{project_id}/balance",
                "get",
                "customerBearer",
            ),
            ("/v1/sessions/{session_id}/jobs/next", "get", "deviceBearer"),
            (
                "/v1/sessions/{session_id}/gpu-inventory",
                "post",
                "deviceBearer",
            ),
        ] {
            let operation = paths.get(path).unwrap().get(method).unwrap();
            let security = operation["security"].as_array().unwrap();
            assert!(
                security.iter().any(|entry| entry[scheme].is_array()),
                "{method} {path} must document {scheme}"
            );
        }

        for (path, method) in [
            ("/v1/providers", "post"),
            (
                "/v1/billing/projects/{project_id}/pix/payment-intents",
                "post",
            ),
            ("/v1/customer/projects/{project_id}/reservations", "post"),
            ("/v1/jobs", "post"),
        ] {
            let operation = paths.get(path).unwrap().get(method).unwrap();
            let parameters = operation["parameters"].as_array().unwrap();
            let idempotency = parameters
                .iter()
                .find(|parameter| {
                    parameter["name"] == "Idempotency-Key"
                        || parameter["$ref"] == "#/components/parameters/IdempotencyKey"
                })
                .unwrap();
            let schema = if idempotency["$ref"] == "#/components/parameters/IdempotencyKey" {
                &document["components"]["parameters"]["IdempotencyKey"]["schema"]
            } else {
                &idempotency["schema"]
            };
            assert_eq!(schema["type"], "string");
            assert_eq!(schema["minLength"], 1);
            assert_eq!(schema["maxLength"], 128);
            assert_eq!(schema["pattern"], "^[!-~]+$");
        }
    }

    #[test]
    fn openapi_documents_bn18_admin_customer_authorization_boundaries() {
        fn assert_single_scheme(operation: &serde_json::Value, expected: &str) {
            let security = operation["security"].as_array().expect("security array");
            assert_eq!(security.len(), 1);
            let entry = security[0].as_object().expect("security object");
            assert_eq!(entry.len(), 1);
            assert!(
                entry
                    .get(expected)
                    .and_then(|value| value.as_array())
                    .is_some(),
                "operation must document only {expected}"
            );
        }

        fn assert_no_idempotency_header(operation: &serde_json::Value) {
            if let Some(parameters) = operation["parameters"].as_array() {
                assert!(
                    !parameters.iter().any(|parameter| {
                        parameter["name"] == "Idempotency-Key"
                            || parameter["$ref"] == "#/components/parameters/IdempotencyKey"
                    }),
                    "operation must not document Idempotency-Key unless runtime requires it"
                );
            }
        }

        let document = document();
        let paths = document["paths"].as_object().unwrap();
        for (path, method) in [
            ("/v1/marketplace/listings/{listing_id}/price", "post"),
            (
                "/v1/billing/pix/payment-intents/{payment_intent_id}/confirm",
                "post",
            ),
            ("/v1/billing/reservations/{reservation_id}/settle", "post"),
            ("/v1/billing/invoices/{invoice_id}", "get"),
            ("/v1/billing/providers/{provider_id}/balance", "get"),
            ("/v1/billing/providers/{provider_id}/ledger", "get"),
            ("/v1/billing/providers/{provider_id}/payout-account", "post"),
            ("/v1/billing/providers/{provider_id}/payouts", "post"),
        ] {
            let operation = paths.get(path).unwrap().get(method).unwrap();
            assert_single_scheme(operation, "adminBearer");
            assert_no_idempotency_header(operation);
        }

        let pix_create = &paths["/v1/billing/projects/{project_id}/pix/payment-intents"]["post"];
        assert_single_scheme(pix_create, "customerBearer");
        assert!(
            pix_create["description"]
                .as_str()
                .unwrap()
                .contains("billing:write")
        );

        for (path, method) in [
            ("/v1/billing/projects/{project_id}/balance", "get"),
            ("/v1/billing/projects/{project_id}/ledger", "get"),
        ] {
            let operation = paths.get(path).unwrap().get(method).unwrap();
            assert_single_scheme(operation, "customerBearer");
            assert!(
                operation["description"]
                    .as_str()
                    .unwrap()
                    .contains("billing:read")
            );
            assert_no_idempotency_header(operation);
        }
    }
    #[test]
    fn openapi_documents_bn01_bn11_request_response_schemas() {
        fn request_schema_ref<'a>(
            paths: &'a serde_json::Map<String, serde_json::Value>,
            path: &str,
            method: &str,
        ) -> &'a str {
            paths[path][method]["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .expect("request schema ref")
        }

        fn response_schema_ref<'a>(
            paths: &'a serde_json::Map<String, serde_json::Value>,
            path: &str,
            method: &str,
            status: &str,
        ) -> &'a str {
            paths[path][method]["responses"][status]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .expect("response schema ref")
        }

        let document = document();
        let schemas = document["components"]["schemas"].as_object().unwrap();
        for schema in [
            "HealthResponse",
            "ReadyResponse",
            "CreateProviderRequest",
            "ProviderEnvelope",
            "IssueEnrollmentTokenResponse",
            "StartEnrollmentRequest",
            "StartEnrollmentResponse",
            "EnrollmentProofRequest",
            "EnrollmentProofResponse",
            "ListProviderDevicesResponse",
            "DeviceCredentialResponse",
            "StartKeyRotationRequest",
            "StartKeyRotationResponse",
            "KeyRotationProofRequest",
            "KeyRotationProofResponse",
            "DeviceRevocationResponse",
            "StartRemoteSessionRequest",
            "StartRemoteSessionResponse",
            "RemoteSessionRecord",
            "HeartbeatControlMessage",
            "HeartbeatReceipt",
            "RemoteSessionRevocationResponse",
            "SignedTelemetryBatch",
            "TelemetryBatchControlMessage",
            "TelemetryBatchReceipt",
            "LatestTelemetryResponse",
            "SubmitEvidenceRequest",
            "SubmitEvidenceResponse",
            "ListEvidenceResponse",
            "RevokeEvidenceRequest",
            "RevokeEvidenceResponse",
            "IssueProofChallengeRequest",
            "IssueProofChallengeResponse",
            "NextProofChallengeResponse",
            "SignedProofCapabilityResponse",
            "SubmitProofChallengeResponse",
            "RunVerificationSweepRequest",
            "RunVerificationSweepResponse",
            "ListVerificationStatesResponse",
            "SubmitNetworkProbeObservationRequest",
            "SubmitNetworkProbeObservationResponse",
            "ListNetworkProbeObservationsResponse",
            "ListProviderNetworkStatesResponse",
            "RunTrustSweepRequest",
            "RunTrustSweepResponse",
            "ListProviderTrustStatesResponse",
            "ListAntifraudEventsResponse",
            "UpsertBenchmarkProfileRequest",
            "UpsertBenchmarkProfileResponse",
            "ListBenchmarkProfilesResponse",
            "SignedBenchmarkResult",
            "SubmitBenchmarkResultResponse",
            "ListProviderBenchmarkResultsResponse",
            "UpsertWorkloadPolicyRequest",
            "UpsertWorkloadPolicyResponse",
            "ListWorkloadPoliciesResponse",
            "RunWorkloadEligibilityRequest",
            "RunWorkloadEligibilityResponse",
            "ListProviderWorkloadEligibilityResponse",
        ] {
            assert!(schemas.contains_key(schema), "missing schema {schema}");
        }

        let paths = document["paths"].as_object().unwrap();
        for (path, method, schema) in [
            ("/v1/providers", "post", "CreateProviderRequest"),
            ("/v1/enrollments", "post", "StartEnrollmentRequest"),
            (
                "/v1/enrollments/{enrollment_id}/proof",
                "post",
                "EnrollmentProofRequest",
            ),
            (
                "/v1/devices/{device_id}/key-rotations",
                "post",
                "StartKeyRotationRequest",
            ),
            (
                "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof",
                "post",
                "KeyRotationProofRequest",
            ),
            ("/v1/sessions", "post", "StartRemoteSessionRequest"),
            (
                "/v1/sessions/{session_id}/heartbeats",
                "post",
                "HeartbeatControlMessage",
            ),
            (
                "/v1/sessions/{session_id}/telemetry-batches",
                "post",
                "TelemetryBatchControlMessage",
            ),
            (
                "/v1/sessions/{session_id}/evidence-records",
                "post",
                "SubmitEvidenceRequest",
            ),
            (
                "/v1/evidence-records/{evidence_id}/revoke",
                "post",
                "RevokeEvidenceRequest",
            ),
            ("/v1/challenges", "post", "IssueProofChallengeRequest"),
            (
                "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
                "post",
                "SignedProofCapabilityResponse",
            ),
            (
                "/v1/verification/sweep",
                "post",
                "RunVerificationSweepRequest",
            ),
            (
                "/v1/network-probes/observations",
                "post",
                "SubmitNetworkProbeObservationRequest",
            ),
            ("/v1/trust/sweep", "post", "RunTrustSweepRequest"),
            (
                "/v1/benchmark-profiles",
                "post",
                "UpsertBenchmarkProfileRequest",
            ),
            (
                "/v1/sessions/{session_id}/benchmark-results",
                "post",
                "SignedBenchmarkResult",
            ),
            (
                "/v1/workload-policies",
                "post",
                "UpsertWorkloadPolicyRequest",
            ),
            (
                "/v1/workload-eligibility/sweep",
                "post",
                "RunWorkloadEligibilityRequest",
            ),
        ] {
            assert_eq!(
                request_schema_ref(paths, path, method),
                format!("#/components/schemas/{schema}"),
                "wrong request schema for {method} {path}"
            );
        }

        for (path, method, status, schema) in [
            ("/health", "get", "200", "HealthResponse"),
            ("/ready", "get", "200", "ReadyResponse"),
            ("/v1/providers", "post", "201", "ProviderEnvelope"),
            (
                "/v1/providers/{provider_id}",
                "get",
                "200",
                "ProviderEnvelope",
            ),
            (
                "/v1/providers/{provider_id}/enrollment-tokens",
                "post",
                "201",
                "IssueEnrollmentTokenResponse",
            ),
            ("/v1/enrollments", "post", "202", "StartEnrollmentResponse"),
            (
                "/v1/enrollments/{enrollment_id}/proof",
                "post",
                "201",
                "EnrollmentProofResponse",
            ),
            (
                "/v1/providers/{provider_id}/devices",
                "get",
                "200",
                "ListProviderDevicesResponse",
            ),
            (
                "/v1/devices/{device_id}/credentials",
                "post",
                "201",
                "DeviceCredentialResponse",
            ),
            (
                "/v1/devices/{device_id}/key-rotations",
                "post",
                "202",
                "StartKeyRotationResponse",
            ),
            (
                "/v1/devices/{device_id}/key-rotations/{rotation_id}/proof",
                "post",
                "200",
                "KeyRotationProofResponse",
            ),
            (
                "/v1/devices/{device_id}/revoke",
                "post",
                "200",
                "DeviceRevocationResponse",
            ),
            ("/v1/sessions", "post", "201", "StartRemoteSessionResponse"),
            (
                "/v1/sessions/{session_id}",
                "get",
                "200",
                "RemoteSessionRecord",
            ),
            (
                "/v1/sessions/{session_id}/heartbeats",
                "post",
                "200",
                "HeartbeatReceipt",
            ),
            (
                "/v1/sessions/{session_id}/revoke",
                "post",
                "200",
                "RemoteSessionRevocationResponse",
            ),
            (
                "/v1/sessions/{session_id}/telemetry-batches",
                "post",
                "200",
                "TelemetryBatchReceipt",
            ),
            (
                "/v1/sessions/{session_id}/telemetry/latest",
                "get",
                "200",
                "LatestTelemetryResponse",
            ),
            (
                "/v1/sessions/{session_id}/evidence-records",
                "post",
                "201",
                "SubmitEvidenceResponse",
            ),
            (
                "/v1/providers/{provider_id}/evidence-records",
                "get",
                "200",
                "ListEvidenceResponse",
            ),
            (
                "/v1/evidence-records/{evidence_id}",
                "get",
                "200",
                "EvidenceRecord",
            ),
            (
                "/v1/evidence-records/{evidence_id}/revoke",
                "post",
                "200",
                "RevokeEvidenceResponse",
            ),
            (
                "/v1/challenges",
                "post",
                "201",
                "IssueProofChallengeResponse",
            ),
            (
                "/v1/sessions/{session_id}/challenges/next",
                "get",
                "200",
                "NextProofChallengeResponse",
            ),
            (
                "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
                "post",
                "200",
                "SubmitProofChallengeResponse",
            ),
            (
                "/v1/verification/sweep",
                "post",
                "202",
                "RunVerificationSweepResponse",
            ),
            (
                "/v1/providers/{provider_id}/verification-states",
                "get",
                "200",
                "ListVerificationStatesResponse",
            ),
            (
                "/v1/network-probes/observations",
                "post",
                "201",
                "SubmitNetworkProbeObservationResponse",
            ),
            (
                "/v1/providers/{provider_id}/network-probes",
                "get",
                "200",
                "ListNetworkProbeObservationsResponse",
            ),
            (
                "/v1/providers/{provider_id}/network-state",
                "get",
                "200",
                "ListProviderNetworkStatesResponse",
            ),
            ("/v1/trust/sweep", "post", "202", "RunTrustSweepResponse"),
            (
                "/v1/providers/{provider_id}/trust-states",
                "get",
                "200",
                "ListProviderTrustStatesResponse",
            ),
            (
                "/v1/providers/{provider_id}/antifraud-events",
                "get",
                "200",
                "ListAntifraudEventsResponse",
            ),
            (
                "/v1/benchmark-profiles",
                "post",
                "201",
                "UpsertBenchmarkProfileResponse",
            ),
            (
                "/v1/benchmark-profiles",
                "get",
                "200",
                "ListBenchmarkProfilesResponse",
            ),
            (
                "/v1/sessions/{session_id}/benchmark-results",
                "post",
                "201",
                "SubmitBenchmarkResultResponse",
            ),
            (
                "/v1/providers/{provider_id}/benchmark-results",
                "get",
                "200",
                "ListProviderBenchmarkResultsResponse",
            ),
            (
                "/v1/workload-policies",
                "post",
                "201",
                "UpsertWorkloadPolicyResponse",
            ),
            (
                "/v1/workload-policies",
                "get",
                "200",
                "ListWorkloadPoliciesResponse",
            ),
            (
                "/v1/workload-eligibility/sweep",
                "post",
                "202",
                "RunWorkloadEligibilityResponse",
            ),
            (
                "/v1/providers/{provider_id}/workload-eligibility",
                "get",
                "200",
                "ListProviderWorkloadEligibilityResponse",
            ),
        ] {
            assert_eq!(
                response_schema_ref(paths, path, method, status),
                format!("#/components/schemas/{schema}"),
                "wrong {status} response schema for {method} {path}"
            );
        }

        let required = schemas["StartEnrollmentRequest"]["required"]
            .as_array()
            .unwrap();
        assert!(required.iter().any(|value| value == "enrollment_token"));
        assert!(required.iter().any(|value| value == "public_key"));
        assert!(
            schemas["StartEnrollmentRequest"]["description"]
                .as_str()
                .unwrap()
                .contains("burd-protocol/control-plane")
        );

        let session_required = schemas["StartRemoteSessionResponse"]["required"]
            .as_array()
            .unwrap();
        assert!(session_required.iter().any(|value| value == "resume_token"));
    }
    #[test]
    fn openapi_protocol_examples_parse_into_burd_protocol_contracts() {
        fn example_value(document: &serde_json::Value, name: &str) -> serde_json::Value {
            document["components"]["examples"][name]["value"].clone()
        }

        let document = document();
        let start_enrollment: burd_protocol::StartEnrollmentRequest =
            serde_json::from_value(example_value(&document, "StartEnrollmentRequestExample"))
                .unwrap();
        assert_eq!(start_enrollment.key_algorithm, "ed25519");
        assert_eq!(
            start_enrollment.registration_payload["secrets_included"],
            false
        );
        let _: burd_protocol::StartEnrollmentResponse =
            serde_json::from_value(example_value(&document, "StartEnrollmentResponseExample"))
                .unwrap();
        let _: burd_protocol::EnrollmentProofRequest =
            serde_json::from_value(example_value(&document, "EnrollmentProofRequestExample"))
                .unwrap();
        let _: burd_protocol::EnrollmentProofResponse =
            serde_json::from_value(example_value(&document, "EnrollmentProofResponseExample"))
                .unwrap();

        let start_session: burd_protocol::StartRemoteSessionRequest =
            serde_json::from_value(example_value(&document, "StartRemoteSessionRequestExample"))
                .unwrap();
        assert_eq!(start_session.provider_id, "provider_example_001");
        let _: burd_protocol::StartRemoteSessionResponse = serde_json::from_value(example_value(
            &document,
            "StartRemoteSessionResponseExample",
        ))
        .unwrap();
        let heartbeat: burd_protocol::ClientControlMessage =
            serde_json::from_value(example_value(&document, "HeartbeatControlMessageExample"))
                .unwrap();
        assert_eq!(heartbeat.message_type, "heartbeat");
        let heartbeat_payload: burd_protocol::HeartbeatPayload =
            serde_json::from_value(heartbeat.payload).unwrap();
        assert_eq!(
            heartbeat_payload.hardware_fingerprint,
            "sha256:example-fingerprint"
        );
        let _: burd_protocol::HeartbeatReceipt =
            serde_json::from_value(example_value(&document, "HeartbeatReceiptExample")).unwrap();

        let evidence: burd_protocol::SubmitEvidenceRequest =
            serde_json::from_value(example_value(&document, "SubmitEvidenceRequestExample"))
                .unwrap();
        assert_eq!(evidence.signed_report.key_algorithm, "ed25519");
        let _: burd_protocol::SubmitEvidenceResponse =
            serde_json::from_value(example_value(&document, "SubmitEvidenceResponseExample"))
                .unwrap();

        let issue: burd_protocol::IssueProofChallengeRequest = serde_json::from_value(
            example_value(&document, "IssueProofChallengeRequestExample"),
        )
        .unwrap();
        assert_eq!(issue.required_backend, "cuda");
        let _: burd_protocol::IssueProofChallengeResponse = serde_json::from_value(example_value(
            &document,
            "IssueProofChallengeResponseExample",
        ))
        .unwrap();
        let signed: burd_protocol::SignedProofCapabilityResponse = serde_json::from_value(
            example_value(&document, "SignedProofCapabilityResponseExample"),
        )
        .unwrap();
        assert_eq!(signed.payload.backend, "cuda");
        let _: burd_protocol::SubmitProofChallengeResponse = serde_json::from_value(example_value(
            &document,
            "SubmitProofChallengeResponseExample",
        ))
        .unwrap();

        let serialized = document["components"]["examples"].to_string();
        for forbidden in ["private_key", "password", "postgres://"] {
            assert!(
                !serialized.contains(forbidden),
                "protocol examples must not contain {forbidden}"
            );
        }
    }

    #[test]
    fn openapi_wires_protocol_examples_to_high_risk_request_and_response_contracts() {
        fn request_example_ref<'a>(
            paths: &'a serde_json::Map<String, serde_json::Value>,
            path: &str,
            method: &str,
            example_key: &str,
        ) -> &'a str {
            paths[path][method]["requestBody"]["content"]["application/json"]["examples"]
                [example_key]["$ref"]
                .as_str()
                .expect("request example ref")
        }

        fn response_example_ref<'a>(
            paths: &'a serde_json::Map<String, serde_json::Value>,
            path: &str,
            method: &str,
            status: &str,
            example_key: &str,
        ) -> &'a str {
            paths[path][method]["responses"][status]["content"]["application/json"]["examples"]
                [example_key]["$ref"]
                .as_str()
                .expect("response example ref")
        }

        let document = document();
        let paths = document["paths"].as_object().unwrap();
        for (path, method, example_key, component_example) in [
            (
                "/v1/enrollments",
                "post",
                "start_enrollment",
                "StartEnrollmentRequestExample",
            ),
            (
                "/v1/enrollments/{enrollment_id}/proof",
                "post",
                "enrollment_proof",
                "EnrollmentProofRequestExample",
            ),
            (
                "/v1/sessions",
                "post",
                "start_remote_session",
                "StartRemoteSessionRequestExample",
            ),
            (
                "/v1/sessions/{session_id}/heartbeats",
                "post",
                "heartbeat",
                "HeartbeatControlMessageExample",
            ),
            (
                "/v1/sessions/{session_id}/evidence-records",
                "post",
                "submit_evidence",
                "SubmitEvidenceRequestExample",
            ),
            (
                "/v1/challenges",
                "post",
                "issue_proof_challenge",
                "IssueProofChallengeRequestExample",
            ),
            (
                "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
                "post",
                "signed_proof_response",
                "SignedProofCapabilityResponseExample",
            ),
        ] {
            assert_eq!(
                request_example_ref(paths, path, method, example_key),
                format!("#/components/examples/{component_example}"),
                "wrong request example for {method} {path}"
            );
        }

        for (path, method, status, example_key, component_example) in [
            (
                "/v1/enrollments",
                "post",
                "202",
                "start_enrollment",
                "StartEnrollmentResponseExample",
            ),
            (
                "/v1/enrollments/{enrollment_id}/proof",
                "post",
                "201",
                "enrollment_proof",
                "EnrollmentProofResponseExample",
            ),
            (
                "/v1/sessions",
                "post",
                "201",
                "start_remote_session",
                "StartRemoteSessionResponseExample",
            ),
            (
                "/v1/sessions/{session_id}/heartbeats",
                "post",
                "200",
                "heartbeat",
                "HeartbeatReceiptExample",
            ),
            (
                "/v1/sessions/{session_id}/evidence-records",
                "post",
                "201",
                "submit_evidence",
                "SubmitEvidenceResponseExample",
            ),
            (
                "/v1/challenges",
                "post",
                "201",
                "issue_proof_challenge",
                "IssueProofChallengeResponseExample",
            ),
            (
                "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
                "post",
                "200",
                "signed_proof_response",
                "SubmitProofChallengeResponseExample",
            ),
        ] {
            assert_eq!(
                response_example_ref(paths, path, method, status, example_key),
                format!("#/components/examples/{component_example}"),
                "wrong {status} response example for {method} {path}"
            );
        }
    }
    #[test]
    fn openapi_documents_job_scheduler_reservation_schemas() {
        fn request_schema_ref<'a>(
            paths: &'a serde_json::Map<String, serde_json::Value>,
            path: &str,
            method: &str,
        ) -> &'a str {
            paths[path][method]["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .expect("request schema ref")
        }

        fn response_schema_ref<'a>(
            paths: &'a serde_json::Map<String, serde_json::Value>,
            path: &str,
            method: &str,
            status: &str,
        ) -> &'a str {
            paths[path][method]["responses"][status]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .expect("response schema ref")
        }

        let document = document();
        let schemas = document["components"]["schemas"].as_object().unwrap();
        for schema in [
            "JobArtifact",
            "CreateJobRequest",
            "JobRecord",
            "CreateJobResponse",
            "JobResponse",
            "ListJobsResponse",
            "JobDataPlaneGrant",
            "ProviderJobExecutionState",
            "ProviderJobRuntimePolicy",
            "ProviderJobCancellationPolicy",
            "ProviderJobCleanupPolicy",
            "ProviderJobExecutionSpec",
            "NextJobResponse",
            "JobEventRequest",
            "JobEventResponse",
            "SubmitJobResultRequest",
            "SubmitJobResultResponse",
            "CancelJobRequest",
            "JobLeaseRecord",
            "RunSchedulerRequest",
            "RunSchedulerResponse",
            "ListJobLeasesResponse",
            "CreateReservationRequest",
            "CancelReservationRequest",
            "MarketplaceReservationRecord",
            "MarketplaceReservationResponse",
            "ListMarketplaceReservationsResponse",
        ] {
            assert!(schemas.contains_key(schema), "missing schema {schema}");
        }

        assert_eq!(
            schemas["NextJobResponse"]["properties"]["execution"]["oneOf"][0]["$ref"],
            "#/components/schemas/ProviderJobExecutionSpec"
        );

        let paths = document["paths"].as_object().unwrap();
        assert_eq!(
            request_schema_ref(paths, "/v1/jobs", "post"),
            "#/components/schemas/CreateJobRequest"
        );
        assert_eq!(
            response_schema_ref(paths, "/v1/jobs", "post", "201"),
            "#/components/schemas/CreateJobResponse"
        );
        assert_eq!(
            response_schema_ref(paths, "/v1/providers/{provider_id}/jobs", "get", "200"),
            "#/components/schemas/ListJobsResponse"
        );
        assert_eq!(
            request_schema_ref(paths, "/v1/scheduler/run", "post"),
            "#/components/schemas/RunSchedulerRequest"
        );
        assert_eq!(
            response_schema_ref(paths, "/v1/scheduler/run", "post", "202"),
            "#/components/schemas/RunSchedulerResponse"
        );
        assert_eq!(
            response_schema_ref(paths, "/v1/jobs/{job_id}/leases", "get", "200"),
            "#/components/schemas/ListJobLeasesResponse"
        );
        assert_eq!(
            request_schema_ref(
                paths,
                "/v1/sessions/{session_id}/jobs/{job_id}/events",
                "post"
            ),
            "#/components/schemas/JobEventRequest"
        );
        assert_eq!(
            response_schema_ref(
                paths,
                "/v1/sessions/{session_id}/jobs/{job_id}/result",
                "post",
                "200",
            ),
            "#/components/schemas/SubmitJobResultResponse"
        );
        assert_eq!(
            request_schema_ref(
                paths,
                "/v1/customer/projects/{project_id}/reservations",
                "post"
            ),
            "#/components/schemas/CreateReservationRequest"
        );
        assert_eq!(
            response_schema_ref(
                paths,
                "/v1/customer/projects/{project_id}/reservations",
                "get",
                "200",
            ),
            "#/components/schemas/ListMarketplaceReservationsResponse"
        );
        assert_eq!(
            request_schema_ref(
                paths,
                "/v1/customer/reservations/{reservation_id}/cancel",
                "post"
            ),
            "#/components/schemas/CancelReservationRequest"
        );
    }

    #[test]
    fn openapi_documents_payout_status_without_transition_endpoints() {
        let document = document();
        let schemas = document["components"]["schemas"].as_object().unwrap();
        let payout_status = schemas["ProviderPayoutRecord"]["properties"]["status"]["enum"]
            .as_array()
            .unwrap();
        for status in ["held", "approved", "paid", "failed", "cancelled"] {
            assert!(
                payout_status.iter().any(|value| value == status),
                "missing payout status {status}"
            );
        }
        assert!(
            schemas["ProviderPayoutRecord"]["properties"]["status"]["description"]
                .as_str()
                .unwrap()
                .contains("reserved for future")
        );
        let paths = document["paths"].as_object().unwrap();
        for path in [
            "/v1/billing/providers/{provider_id}/payouts/{payout_id}/paid",
            "/v1/billing/providers/{provider_id}/payouts/{payout_id}/failed",
            "/v1/billing/providers/{provider_id}/payouts/{payout_id}/cancel",
        ] {
            assert!(
                !paths.contains_key(path),
                "unimplemented payout transition endpoint must not be documented"
            );
        }
        assert_eq!(
            paths["/v1/billing/providers/{provider_id}/payouts"]["post"]["responses"]["201"]["content"]
                ["application/json"]["schema"]["$ref"],
            "#/components/schemas/ProviderPayoutResponse"
        );
    }
    #[test]
    fn openapi_documents_bn18_billing_error_contracts() {
        let document = document();
        let components = &document["components"];
        let error_schema = &components["schemas"]["ErrorEnvelope"];
        assert_eq!(error_schema["required"][0], "error");
        assert_eq!(
            error_schema["properties"]["error"]["properties"]["retry_after_seconds"]["type"][1],
            "null"
        );
        assert!(components["responses"]["IdempotencyConflict"].is_object());
        assert!(components["examples"]["BillingInsufficientBalance"].is_object());
        assert!(components["examples"]["BillingUsageAlreadyInvoiced"].is_object());
        assert!(components["examples"]["PayoutPolicyBlocked"].is_object());

        let paths = document["paths"].as_object().unwrap();
        let pix_create = &paths["/v1/billing/projects/{project_id}/pix/payment-intents"]["post"];
        assert_eq!(
            pix_create["parameters"][0]["$ref"],
            "#/components/parameters/IdempotencyKey"
        );
        assert_eq!(
            pix_create["responses"]["409"]["$ref"],
            "#/components/responses/IdempotencyConflict"
        );

        let pix_confirm =
            &paths["/v1/billing/pix/payment-intents/{payment_intent_id}/confirm"]["post"];
        assert_eq!(
            pix_confirm["responses"]["409"]["$ref"],
            "#/components/responses/Conflict"
        );

        let settlement = &paths["/v1/billing/reservations/{reservation_id}/settle"]["post"];
        let settlement_examples =
            settlement["responses"]["409"]["content"]["application/json"]["examples"]
                .as_object()
                .unwrap();
        assert!(settlement_examples.contains_key("insufficient_balance"));
        assert!(settlement_examples.contains_key("usage_already_invoiced"));

        let payout = &paths["/v1/billing/providers/{provider_id}/payouts"]["post"];
        assert!(
            payout["description"]
                .as_str()
                .unwrap()
                .contains("does not call a bank")
        );
        assert!(
            payout["responses"]["409"]["content"]["application/json"]["examples"]
                .as_object()
                .unwrap()
                .contains_key("policy_blocked")
        );
    }
}
