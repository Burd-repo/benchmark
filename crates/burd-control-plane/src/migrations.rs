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
}
