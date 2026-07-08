#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: &'static str,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: "0001",
    name: "initial_control_plane",
    sql: include_str!("../migrations/0001_initial.sql"),
}];

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
}
