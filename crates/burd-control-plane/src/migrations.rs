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
}
