#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: &'static str,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: "0001",
        name: "initial_control_plane",
        sql: include_str!("../migrations/0001_initial.sql"),
    },
    Migration {
        version: "0002",
        name: "provider_enrollment",
        sql: include_str!("../migrations/0002_provider_enrollment.sql"),
    },
    Migration {
        version: "0003",
        name: "remote_sessions",
        sql: include_str!("../migrations/0003_remote_sessions.sql"),
    },
    Migration {
        version: "0004",
        name: "gpu_telemetry",
        sql: include_str!("../migrations/0004_gpu_telemetry.sql"),
    },
    Migration {
        version: "0005",
        name: "remote_evidence_registry",
        sql: include_str!("../migrations/0005_remote_evidence_registry.sql"),
    },
    Migration {
        version: "0006",
        name: "active_proof_of_capability",
        sql: include_str!("../migrations/0006_active_proof_of_capability.sql"),
    },
    Migration {
        version: "0007",
        name: "recurring_verification_policy",
        sql: include_str!("../migrations/0007_recurring_verification_policy.sql"),
    },
    Migration {
        version: "0008",
        name: "regional_network_probes",
        sql: include_str!("../migrations/0008_regional_network_probes.sql"),
    },
    Migration {
        version: "0009",
        name: "global_trust_antifraud",
        sql: include_str!("../migrations/0009_global_trust_antifraud.sql"),
    },
    Migration {
        version: "0010",
        name: "benchmark_profiles_v2",
        sql: include_str!("../migrations/0010_benchmark_profiles_v2.sql"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_migration_declares_bn01_tables() {
        let sql = MIGRATIONS[0].sql;
        for table in [
            "users",
            "providers",
            "devices",
            "provider_identities",
            "provider_public_keys",
            "hardware_snapshots",
            "evidence_records",
            "provider_sessions",
            "audit_events",
        ] {
            assert!(sql.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")));
        }
    }

    #[test]
    fn enrollment_migration_declares_bn02_tables() {
        let sql = MIGRATIONS[1].sql;
        for table in [
            "enrollment_tokens",
            "device_enrollments",
            "device_credentials",
            "key_rotation_challenges",
        ] {
            assert!(sql.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")));
        }
    }

    #[test]
    fn remote_session_migration_declares_bn03_heartbeats() {
        let sql = MIGRATIONS[2].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS session_heartbeats"));
        assert!(sql.contains("idx_provider_sessions_active_device"));
    }

    #[test]
    fn gpu_telemetry_migration_declares_bn04_tables() {
        let sql = MIGRATIONS[3].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS telemetry_batches"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS gpu_telemetry_samples"));
    }

    #[test]
    fn remote_evidence_migration_extends_evidence_records() {
        let sql = MIGRATIONS[4].sql;
        assert!(sql.contains("ADD COLUMN IF NOT EXISTS session_id"));
        assert!(sql.contains("idx_evidence_records_evidence_hash"));
    }

    #[test]
    fn active_proof_migration_declares_challenge_registry() {
        let sql = MIGRATIONS[5].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS proof_challenges"));
        assert!(sql.contains("response_hash TEXT UNIQUE"));
        assert!(sql.contains("idx_proof_challenges_session_status"));
    }

    #[test]
    fn recurring_verification_migration_declares_policy_state() {
        let sql = MIGRATIONS[6].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS provider_verification_states"));
        assert!(sql.contains("ADD COLUMN IF NOT EXISTS trigger_reason"));
        assert!(sql.contains("idx_provider_verification_states_status_due"));
    }

    #[test]
    fn regional_network_probe_migration_declares_probe_registry() {
        let sql = MIGRATIONS[7].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS network_probe_observations"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS provider_network_states"));
        assert!(sql.contains("UNIQUE(session_id, probe_id, observed_at)"));
        assert!(sql.contains("idx_network_probe_observations_provider_time"));
    }

    #[test]
    fn global_trust_antifraud_migration_declares_state_and_events() {
        let sql = MIGRATIONS[8].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS provider_trust_states"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS antifraud_events"));
        assert!(sql.contains("UNIQUE(provider_id, device_id, event_type, reason)"));
        assert!(sql.contains("idx_provider_trust_states_status_score"));
    }

    #[test]
    fn benchmark_profiles_v2_migration_declares_profiles_and_results() {
        let sql = MIGRATIONS[9].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS benchmark_profiles"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS benchmark_results"));
        assert!(sql.contains("backend TEXT NOT NULL"));
        assert!(sql.contains("UNIQUE(provider_id, device_id, run_id)"));
        assert!(sql.contains("idx_benchmark_results_provider_time"));
    }
}
