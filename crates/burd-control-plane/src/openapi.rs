pub fn document() -> serde_json::Value {
    serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Burd Control Plane API",
            "version": "v1",
            "description": "BN-04 control plane API for provider identity, remote sessions, signed GPU telemetry ingestion, outbound WebSocket control channels, revocation, health, readiness, and audit-backed persistence."
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
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_lists_bn04_identity_session_and_telemetry_endpoints() {
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
        ] {
            assert!(paths.contains_key(path), "missing OpenAPI path {path}");
        }
        assert!(document["components"]["securitySchemes"]["adminBearer"].is_object());
        assert!(document["components"]["securitySchemes"]["deviceBearer"].is_object());
    }
}
