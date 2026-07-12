pub fn document() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Burd Control Plane API",
            "version": "v1",
            "description": "BN-10 control plane API for provider identity, remote sessions, signed GPU telemetry, remote evidence registry, active proof-of-capability challenges, recurring/risk-based verification state, regional network probes, global trust/antifraud state, versioned benchmark profiles, signed benchmark results, outbound WebSocket control channels, revocation, health, readiness, and audit-backed persistence."
        },
        "components": {
            "securitySchemes": {
                "adminBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Bootstrap admin credential configured by BURD_CONTROL_ADMIN_TOKEN."
                },
                "deviceBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Short-lived device credential issued after enrollment proof."
                }
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
            "/v1/providers": {
                "post": {
                    "summary": "Create a provider registry record",
                    "security": [{ "adminBearer": [] }],
                    "parameters": [
                        {
                            "name": "Idempotency-Key",
                            "in": "header",
                            "required": true,
                            "schema": { "type": "string" }
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
                        "400": { "description": "invalid sweep request" },
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_lists_bn10_identity_session_telemetry_evidence_challenge_verification_network_trust_and_benchmark_endpoints()
     {
        let document = document();
        let paths = document["paths"].as_object().unwrap();
        for path in [
            "/health",
            "/ready",
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
            "/v1/evidence-records/{evidence_id}",
            "/v1/evidence-records/{evidence_id}/revoke",
            "/v1/network-probes/observations",
            "/v1/providers/{provider_id}/network-probes",
            "/v1/providers/{provider_id}/network-state",
            "/v1/benchmark-profiles",
            "/v1/sessions/{session_id}/benchmark-results",
            "/v1/providers/{provider_id}/benchmark-results",
            "/v1/trust/sweep",
            "/v1/providers/{provider_id}/trust-states",
            "/v1/providers/{provider_id}/antifraud-events",
            "/v1/verification/sweep",
            "/v1/providers/{provider_id}/verification-states",
            "/v1/challenges",
            "/v1/challenges/{challenge_id}",
            "/v1/sessions/{session_id}/challenges/next",
            "/v1/sessions/{session_id}/challenges/{challenge_id}/response",
        ] {
            assert!(paths.contains_key(path), "missing OpenAPI path {path}");
        }
        assert!(document["components"]["securitySchemes"]["adminBearer"].is_object());
        assert!(document["components"]["securitySchemes"]["deviceBearer"].is_object());
    }
}
