ALTER TABLE provider_sessions
    ADD COLUMN IF NOT EXISTS negotiated_protocol_version TEXT,
    ADD COLUMN IF NOT EXISTS protocol_negotiation_status TEXT NOT NULL DEFAULT 'legacy_unnegotiated',
    ADD COLUMN IF NOT EXISTS declared_protocol_versions_json TEXT NOT NULL DEFAULT '[]',
    ADD COLUMN IF NOT EXISTS declared_protocol_capabilities_json TEXT NOT NULL DEFAULT '[]',
    ADD COLUMN IF NOT EXISTS accepted_protocol_capabilities_json TEXT NOT NULL DEFAULT '[]',
    ADD COLUMN IF NOT EXISTS protocol_policy_version TEXT,
    ADD COLUMN IF NOT EXISTS protocol_reason_codes_json TEXT NOT NULL DEFAULT '[]',
    ADD COLUMN IF NOT EXISTS protocol_negotiated_at TEXT;

ALTER TABLE provider_sessions
    ADD CONSTRAINT provider_sessions_protocol_status_valid CHECK (
        protocol_negotiation_status IN (
            'accepted', 'upgrade_required', 'incompatible_protocol',
            'missing_capabilities', 'legacy_unnegotiated'
        )
    ),
    ADD CONSTRAINT provider_sessions_protocol_authority_consistent CHECK (
        (protocol_negotiation_status = 'accepted'
            AND negotiated_protocol_version IS NOT NULL
            AND protocol_policy_version IS NOT NULL
            AND protocol_negotiated_at IS NOT NULL)
        OR
        (protocol_negotiation_status <> 'accepted'
            AND negotiated_protocol_version IS NULL)
    );
