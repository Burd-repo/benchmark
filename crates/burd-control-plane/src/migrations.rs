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
    Migration {
        version: "0011",
        name: "workload_eligibility_v2",
        sql: include_str!("../migrations/0011_workload_eligibility_v2.sql"),
    },
    Migration {
        version: "0012",
        name: "job_api_data_plane",
        sql: include_str!("../migrations/0012_job_api_data_plane.sql"),
    },
    Migration {
        version: "0013",
        name: "scheduler_leases",
        sql: include_str!("../migrations/0013_scheduler_leases.sql"),
    },
    Migration {
        version: "0014",
        name: "usage_metering_ledger",
        sql: include_str!("../migrations/0014_usage_metering_ledger.sql"),
    },
    Migration {
        version: "0015",
        name: "marketplace_registry_listings",
        sql: include_str!("../migrations/0015_marketplace_registry_listings.sql"),
    },
    Migration {
        version: "0016",
        name: "customer_accounts_reservations",
        sql: include_str!("../migrations/0016_customer_accounts_reservations.sql"),
    },
    Migration {
        version: "0017",
        name: "billing_pix_payouts",
        sql: include_str!("../migrations/0017_billing_pix_payouts.sql"),
    },
    Migration {
        version: "0018",
        name: "security_hardening_attestation",
        sql: include_str!("../migrations/0018_security_hardening_attestation.sql"),
    },
    Migration {
        version: "0019",
        name: "multi_gpu_inventory",
        sql: include_str!("../migrations/0019_multi_gpu_inventory.sql"),
    },
    Migration {
        version: "0020",
        name: "gpu_inventory_snapshot_uniqueness",
        sql: include_str!("../migrations/0020_gpu_inventory_snapshot_uniqueness.sql"),
    },
    Migration {
        version: "0021",
        name: "unique_billing_usage_invoice",
        sql: include_str!("../migrations/0021_unique_billing_usage_invoice.sql"),
    },
    Migration {
        version: "0022",
        name: "unique_customer_reservation_credit_entries",
        sql: include_str!("../migrations/0022_unique_customer_reservation_credit_entries.sql"),
    },
    Migration {
        version: "0023",
        name: "provider_payout_reconciliation_integrity",
        sql: include_str!("../migrations/0023_provider_payout_reconciliation_integrity.sql"),
    },
    Migration {
        version: "0024",
        name: "refund_dispute_placeholder_integrity",
        sql: include_str!("../migrations/0024_refund_dispute_placeholder_integrity.sql"),
    },
    Migration {
        version: "0025",
        name: "job_artifact_transfers",
        sql: include_str!("../migrations/0025_job_artifact_transfers.sql"),
    },
    Migration {
        version: "0026",
        name: "runtime_capability_verification",
        sql: include_str!("../migrations/0026_runtime_capability_verification.sql"),
    },
    Migration {
        version: "0027",
        name: "runtime_verified_admission",
        sql: include_str!("../migrations/0027_runtime_verified_admission.sql"),
    },
    Migration {
        version: "0028",
        name: "scheduler_runtime_admission",
        sql: include_str!("../migrations/0028_scheduler_runtime_admission.sql"),
    },
    Migration {
        version: "0029",
        name: "gpu_inventory_authoritative_snapshots",
        sql: include_str!("../migrations/0029_gpu_inventory_authoritative_snapshots.sql"),
    },
    Migration {
        version: "0030",
        name: "compute_job_assignment_lease_binding",
        sql: include_str!("../migrations/0030_compute_job_assignment_lease_binding.sql"),
    },
    Migration {
        version: "0031",
        name: "customer_workloads_placement",
        sql: include_str!("../migrations/0031_customer_workloads_placement.sql"),
    },
    Migration {
        version: "0032",
        name: "reservation_workload_binding",
        sql: include_str!("../migrations/0032_reservation_workload_binding.sql"),
    },
    Migration {
        version: "0033",
        name: "customer_artifact_ingress",
        sql: include_str!("../migrations/0033_customer_artifact_ingress.sql"),
    },
    Migration {
        version: "0034",
        name: "customer_job_control",
        sql: include_str!("../migrations/0034_customer_job_control.sql"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use chrono::{Duration, Utc};
    use uuid::Uuid;

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
    fn customer_artifact_ingress_migration_binds_project_artifacts_to_workloads() {
        let sql = MIGRATIONS
            .iter()
            .find(|migration| migration.version == "0033")
            .unwrap()
            .sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS customer_artifacts"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS customer_workload_input_artifacts"));
        assert!(sql.contains("customer_artifacts_sha256_format"));
        assert!(sql.contains("customer_artifacts_ready_state"));
    }

    #[test]
    fn customer_job_control_migration_indexes_owned_status_and_events() {
        let sql = MIGRATIONS.last().unwrap().sql;
        assert!(sql.contains("idx_customer_workloads_project_job"));
        assert!(sql.contains("idx_job_events_customer_projection"));
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

    #[test]
    fn workload_eligibility_v2_migration_declares_policies_and_states() {
        let sql = MIGRATIONS[10].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS workload_policies"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS provider_workload_eligibility"));
        assert!(sql.contains("benchmark_backend TEXT"));
        assert!(sql.contains("vram_total_mib BIGINT"));
        assert!(sql.contains("idx_provider_workload_eligibility_workload_status"));
    }

    #[test]
    fn job_api_data_plane_migration_declares_jobs_and_events() {
        let sql = MIGRATIONS[11].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS compute_jobs"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS job_events"));
        assert!(sql.contains("UNIQUE(provider_id, client_job_id)"));
        assert!(sql.contains("UNIQUE(job_id, sequence)"));
        assert!(sql.contains("idx_compute_jobs_session_status"));
    }
    #[test]
    fn scheduler_leases_migration_declares_job_leases() {
        let sql = MIGRATIONS[12].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS job_leases"));
        assert!(sql.contains("idx_job_leases_active_job"));
        assert!(sql.contains("idx_job_leases_active_gpu"));
        assert!(sql.contains("idx_job_leases_session_status"));
    }

    #[test]
    fn usage_metering_ledger_migration_declares_append_only_ledger() {
        let sql = MIGRATIONS[13].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS usage_ledger_entries"));
        assert!(sql.contains("UNIQUE(job_id, entry_type)"));
        assert!(sql.contains("prevent_usage_ledger_mutation"));
        assert!(sql.contains("usage_ledger_no_update"));
        assert!(sql.contains("idx_usage_ledger_provider_time"));
    }

    #[test]
    fn marketplace_registry_listings_migration_declares_listing_registry() {
        let sql = MIGRATIONS[14].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS marketplace_listings"));
        assert!(
            sql.contains(
                "UNIQUE(provider_id, device_id, workload_type, policy_id, policy_version)"
            )
        );
        assert!(sql.contains("gpu_verified BOOLEAN NOT NULL"));
        assert!(sql.contains("vram_verified BOOLEAN NOT NULL"));
        assert!(sql.contains("idx_marketplace_listings_status_workload"));
    }
    #[test]
    fn billing_pix_payouts_migration_declares_financial_tables() {
        let sql = MIGRATIONS[16].sql;
        for table in [
            "marketplace_listing_prices",
            "pix_payment_intents",
            "billing_invoices",
            "financial_ledger_lines",
            "provider_payout_accounts",
            "provider_payouts",
            "billing_refunds",
            "billing_disputes",
            "billing_reconciliation_events",
        ] {
            assert!(sql.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")));
        }
        assert!(sql.contains("prevent_financial_ledger_mutation"));
        assert!(sql.contains("financial_ledger_no_update"));
        assert!(sql.contains("idx_marketplace_listing_prices_active"));
        assert!(sql.contains("UNIQUE(reservation_id, usage_entry_id)"));
    }
    #[test]
    fn security_hardening_attestation_migration_declares_posture_registry() {
        let sql = MIGRATIONS[17].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS device_security_postures"));
        assert!(sql.contains("posture_hash TEXT NOT NULL UNIQUE"));
        assert!(sql.contains("prevent_device_security_posture_mutation"));
        assert!(sql.contains("device_security_postures_no_update"));
        assert!(sql.contains("idx_device_security_postures_provider_time"));
    }

    #[test]
    fn multi_gpu_inventory_migration_declares_inventory_registry() {
        let sql = MIGRATIONS[18].sql;
        for needle in [
            "device_gpu_inventory",
            "gpu_index",
            "gpu_uuid",
            "verification_json",
        ] {
            assert!(sql.contains(needle));
        }
    }

    #[test]
    fn gpu_inventory_snapshot_uniqueness_migration_allows_per_gpu_snapshot_rows() {
        let sql = MIGRATIONS[19].sql;
        assert!(sql.contains("DROP CONSTRAINT IF EXISTS device_gpu_inventory_inventory_hash_key"));
        assert!(sql.contains("idx_device_gpu_inventory_snapshot_gpu"));
        assert!(sql.contains("inventory_hash, gpu_index"));
        assert!(sql.contains("idx_device_gpu_inventory_hash"));
    }

    #[test]
    fn unique_billing_usage_invoice_migration_prevents_usage_rebilling() {
        let sql = MIGRATIONS[20].sql;
        assert!(sql.contains("idx_billing_invoices_unique_usage_entry"));
        assert!(sql.contains("ON billing_invoices(usage_entry_id)"));
        assert!(sql.contains("WHERE usage_entry_id IS NOT NULL"));
    }
    #[test]
    fn unique_customer_reservation_credit_entries_migration_prevents_duplicate_hold_release() {
        let sql = MIGRATIONS[21].sql;
        assert!(sql.contains("idx_customer_credit_ledger_reservation_entry"));
        assert!(sql.contains("ON customer_credit_ledger_entries(reservation_id, entry_type)"));
        assert!(sql.contains("entry_type IN ('reservation_hold', 'reservation_release')"));
    }

    #[test]
    fn provider_payout_reconciliation_integrity_migration_hardens_financial_tables() {
        let sql = MIGRATIONS[22].sql;
        for needle in [
            "financial_ledger_lines_amount_nonzero",
            "financial_ledger_lines_line_number_positive",
            "provider_payout_accounts_method_pix",
            "provider_payout_accounts_hold_days_range",
            "provider_payouts_status_allowed",
            "provider_payouts_held_requires_hold_until",
            "provider_payouts_paid_requires_external_reference",
            "billing_reconciliation_events_amount_positive",
            "idx_provider_payouts_external_reference",
            "idx_billing_reconciliation_reference",
        ] {
            assert!(sql.contains(needle));
        }
    }
    #[test]
    fn refund_dispute_placeholder_integrity_migration_hardens_placeholders() {
        let sql = MIGRATIONS[23].sql;
        for needle in [
            "billing_refunds_amount_positive",
            "billing_refunds_status_allowed",
            "billing_refunds_reason_not_blank",
            "billing_disputes_hold_amount_positive",
            "billing_disputes_status_allowed",
            "billing_disputes_reason_not_blank",
            "billing_reconciliation_events_status_allowed",
            "idx_billing_disputes_open_invoice",
            "idx_billing_reconciliation_status",
        ] {
            assert!(sql.contains(needle));
        }
    }

    #[test]
    fn job_artifact_transfer_migration_records_verified_uploads() {
        let sql = MIGRATIONS[24].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS job_artifact_uploads"));
        assert!(sql.contains("PRIMARY KEY(job_id, artifact_id)"));
        assert!(sql.contains("sha256 TEXT NOT NULL"));
        assert!(sql.contains("size_bytes BIGINT NOT NULL"));
    }

    #[test]
    fn runtime_capability_verification_migration_declares_authoritative_registry() {
        let sql = MIGRATIONS[25].sql;
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS runtime_verification_challenges"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS provider_runtime_verifications"));
        assert!(sql.contains("runtime_verification_fingerprint TEXT NOT NULL"));
        assert!(sql.contains("idx_provider_runtime_verifications_active_gpu"));
        assert!(sql.contains("response_hash TEXT UNIQUE"));
    }

    #[test]
    fn runtime_verified_admission_migration_binds_keys_and_current_observations() {
        let sql = MIGRATIONS[26].sql;
        assert!(sql.contains("ADD COLUMN IF NOT EXISTS public_key_id"));
        assert!(sql.contains("runtime_admission_fingerprint"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS provider_runtime_observations"));
        assert!(sql.contains("observation_hash TEXT NOT NULL UNIQUE"));
        assert!(sql.contains("signature TEXT NOT NULL"));
        assert!(sql.contains("provider_runtime_observations_no_update"));
        assert!(sql.contains("status = 'superseded'"));
    }

    #[test]
    fn scheduler_runtime_admission_migration_adds_fairness_cursor() {
        let sql = MIGRATIONS[27].sql;
        assert!(sql.contains("scheduler_last_evaluated_at TEXT"));
        assert!(sql.contains("idx_compute_jobs_scheduler_fairness"));
    }

    #[test]
    fn authoritative_gpu_snapshot_migration_preserves_empty_complete_snapshots() {
        let sql = MIGRATIONS[28].sql;
        for needle in [
            "device_gpu_inventory_snapshots",
            "ingest_seq BIGINT GENERATED ALWAYS AS IDENTITY",
            "gpu_count INTEGER NOT NULL CHECK (gpu_count BETWEEN 0 AND 32)",
            "ALTER COLUMN snapshot_id SET NOT NULL",
            "device_gpu_inventory_snapshot_binding_fk",
            "device_gpu_inventory_snapshots_no_update",
            "ORDER BY MIN(server_received_at) ASC, MIN(observed_at) ASC, inventory_hash ASC",
        ] {
            assert!(sql.contains(needle));
        }
    }

    #[test]
    fn compute_job_assignment_lease_migration_binds_exact_acknowledgements() {
        let sql = MIGRATIONS[29].sql;
        for needle in [
            "ADD COLUMN IF NOT EXISTS assignment_lease_id TEXT",
            "compute_jobs_assignment_lease_binding_fk",
            "compute_jobs_active_assignment_lease_check",
            "compute_jobs_queued_assignment_lease_check",
            "idx_compute_jobs_assignment_lease",
        ] {
            assert!(sql.contains(needle));
        }
    }

    #[test]
    fn customer_workload_migration_separates_intent_placement_and_job() {
        let sql = MIGRATIONS[30].sql;
        for needle in [
            "CREATE TABLE IF NOT EXISTS customer_workloads",
            "CREATE TABLE IF NOT EXISTS workload_execution_profiles",
            "CREATE TABLE IF NOT EXISTS compute_placements",
            "UNIQUE(project_id, idempotency_key)",
            "ADD COLUMN IF NOT EXISTS workload_id TEXT UNIQUE",
            "ADD COLUMN IF NOT EXISTS placement_id TEXT UNIQUE",
            "compute_jobs_workload_placement_pair_check",
            "idx_compute_placements_active_supply",
        ] {
            assert!(sql.contains(needle));
        }
    }

    #[test]
    fn reservation_workload_binding_migration_is_exclusive() {
        let sql = MIGRATIONS[31].sql;
        for needle in [
            "ADD COLUMN IF NOT EXISTS reservation_id TEXT",
            "idx_customer_workloads_reservation",
            "idx_compute_placements_reservation",
            "idx_compute_jobs_reservation",
            "status IN ('reserved', 'consumed', 'released', 'cancelled', 'expired')",
            "customer_workloads_reservation_binding_fk",
            "compute_placements_reservation_binding_fk",
            "compute_jobs_reservation_workload_binding_fk",
        ] {
            assert!(sql.contains(needle));
        }
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_authoritative_snapshot_migration_backfills_and_restores_immutability() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!("burd_inventory_migration_{}", Uuid::new_v4().simple());
        let db = Database::new(url, Some(schema)).unwrap();
        let mut client = db.connect().await.unwrap();
        let transaction = client.transaction().await.unwrap();
        for migration in &MIGRATIONS[..28] {
            transaction.batch_execute(migration.sql).await.unwrap();
        }
        let first = Utc::now();
        let second = first + Duration::seconds(1);
        let expires_at = (first + Duration::hours(1)).to_rfc3339();
        let first_text = first.to_rfc3339();
        let second_text = second.to_rfc3339();
        transaction
            .execute(
                "INSERT INTO providers (provider_id, display_name, status, created_at, updated_at) VALUES ('provider_1', 'Migration Provider', 'available', $1, $1)",
                &[&first_text],
            )
            .await
            .unwrap();
        transaction
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ('device_1', 'provider_1', 'machine_1', 'active', $1, $1)",
                &[&first_text],
            )
            .await
            .unwrap();
        transaction
            .execute(
                "INSERT INTO provider_public_keys (public_key_id, provider_id, device_id, public_key, key_algorithm, status, created_at) VALUES ('key_1', 'provider_1', 'device_1', 'public_key', 'ed25519', 'active', $1)",
                &[&first_text],
            )
            .await
            .unwrap();
        transaction
            .execute(
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ('session_1', 'provider_1', 'device_1', 'online', 0, $1, $2, $3)",
                &[&first_text, &expires_at, &"a".repeat(64)],
            )
            .await
            .unwrap();

        for (hash, observed_at, gpus) in [
            (
                "inventory_hash_first",
                first_text.as_str(),
                vec!["GPU-A", "GPU-B"],
            ),
            ("inventory_hash_second", second_text.as_str(), vec!["GPU-C"]),
        ] {
            let payload_json = serde_json::json!({
                "schema_version": "burd-device-gpu-inventory-v1",
                "provider_id": "provider_1",
                "device_id": "device_1",
                "session_id": "session_1",
                "hardware_fingerprint": "a".repeat(64),
                "observed_at": observed_at,
                "gpus": gpus.iter().enumerate().map(|(index, gpu_uuid)| serde_json::json!({
                    "gpu_uuid": gpu_uuid,
                    "gpu_index": index,
                    "backend": "cuda",
                    "pci_vendor_id": "10de",
                    "pci_device_id": "2684",
                    "vram_total_mib": 24576,
                    "status": "active",
                })).collect::<Vec<_>>(),
            })
            .to_string();
            for (index, gpu_uuid) in gpus.iter().enumerate() {
                let row_id = format!("inventory_row_{hash}_{index}");
                let gpu_index = index as i32;
                transaction
                    .execute(
                        "INSERT INTO device_gpu_inventory (inventory_row_id, provider_id, device_id, session_id, schema_version, inventory_hash, public_key_id, signature, canonicalization_version, gpu_uuid, gpu_index, backend, pci_vendor_id, pci_device_id, vram_total_mib, status, observed_at, server_received_at, payload_json, verification_json) VALUES ($1, 'provider_1', 'device_1', 'session_1', 'burd-device-gpu-inventory-v1', $2, 'key_1', 'signature', 'burd-json-c14n-v1', $3, $4, 'cuda', '10de', '2684', 24576, 'active', $5, $5, $6, '{}')",
                        &[&row_id, &hash, &gpu_uuid, &gpu_index, &observed_at, &payload_json],
                    )
                    .await
                    .unwrap();
            }
        }

        transaction.batch_execute(MIGRATIONS[28].sql).await.unwrap();
        transaction.commit().await.unwrap();
        let rows = client
            .query(
                "SELECT inventory_hash, gpu_count FROM device_gpu_inventory_snapshots ORDER BY ingest_seq ASC",
                &[],
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].get::<_, String>("inventory_hash"),
            "inventory_hash_first"
        );
        assert_eq!(rows[0].get::<_, i32>("gpu_count"), 2);
        assert_eq!(
            rows[1].get::<_, String>("inventory_hash"),
            "inventory_hash_second"
        );
        assert_eq!(rows[1].get::<_, i32>("gpu_count"), 1);
        let unbound: i64 = client
            .query_one(
                "SELECT COUNT(*)::BIGINT FROM device_gpu_inventory WHERE snapshot_id IS NULL",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(unbound, 0);
        assert!(
            client
                .execute(
                    "UPDATE device_gpu_inventory SET status = 'inactive' WHERE inventory_row_id = 'inventory_row_inventory_hash_first_0'",
                    &[],
                )
                .await
                .is_err()
        );
        assert!(
            client
                .execute(
                    "UPDATE device_gpu_inventory_snapshots SET gpu_count = 0 WHERE inventory_hash = 'inventory_hash_first'",
                    &[],
                )
                .await
                .is_err()
        );
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn postgres_assignment_lease_migration_backfills_exact_binding() {
        let url = std::env::var("BURD_CONTROL_TEST_DATABASE_URL")
            .expect("BURD_CONTROL_TEST_DATABASE_URL is required for the ignored database test");
        let schema = format!(
            "burd_assignment_lease_migration_{}",
            Uuid::new_v4().simple()
        );
        let db = Database::new(url, Some(schema)).unwrap();
        let mut client = db.connect().await.unwrap();
        let transaction = client.transaction().await.unwrap();
        for migration in &MIGRATIONS[..29] {
            transaction.batch_execute(migration.sql).await.unwrap();
        }
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
        transaction
            .execute(
                "INSERT INTO providers (provider_id, display_name, status, created_at, updated_at) VALUES ('provider_1', 'Assignment Provider', 'available', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        transaction
            .execute(
                "INSERT INTO devices (device_id, provider_id, machine_id, status, created_at, updated_at) VALUES ('device_1', 'provider_1', 'machine_1', 'active', $1, $1)",
                &[&now],
            )
            .await
            .unwrap();
        transaction
            .execute(
                "INSERT INTO provider_sessions (session_id, provider_id, device_id, status, sequence_last, started_at, expires_at, hardware_fingerprint) VALUES ('session_1', 'provider_1', 'device_1', 'online', 0, $1, $2, $3)",
                &[&now, &expires_at, &"a".repeat(64)],
            )
            .await
            .unwrap();
        transaction
            .execute(
                "INSERT INTO compute_jobs (job_id, provider_id, device_id, session_id, schema_version, workload_type, template_id, image_ref, gpu_uuid, backend, status, timeout_seconds, job_credential_hash, job_credential_expires_at, created_at, assigned_at, updated_at) VALUES ('job_1', 'provider_1', 'device_1', 'session_1', 'burd-job-v1', 'llm_batch_inference', 'template_1', $1, 'GPU-A', 'cuda', 'assigned', 300, 'credential_hash', $2, $3, $3, $3)",
                &[&format!("ghcr.io/burd/test@sha256:{}", "b".repeat(64)), &expires_at, &now],
            )
            .await
            .unwrap();
        transaction
            .execute(
                "INSERT INTO job_leases (lease_id, job_id, provider_id, device_id, session_id, schema_version, workload_type, gpu_uuid, status, offered_at, expires_at, failure_reason, created_at, updated_at) VALUES ('lease_1', 'job_1', 'provider_1', 'device_1', 'session_1', 'burd-job-lease-v1', 'llm_batch_inference', 'GPU-A', 'expired', $1, $2, 'lease_ack_timeout', $1, $1)",
                &[&now, &expires_at],
            )
            .await
            .unwrap();

        transaction.batch_execute(MIGRATIONS[29].sql).await.unwrap();
        transaction.commit().await.unwrap();

        let assignment_lease_id: Option<String> = client
            .query_one(
                "SELECT assignment_lease_id FROM compute_jobs WHERE job_id = 'job_1'",
                &[],
            )
            .await
            .unwrap()
            .get("assignment_lease_id");
        assert_eq!(assignment_lease_id.as_deref(), Some("lease_1"));
        assert!(
            client
                .execute(
                    "UPDATE compute_jobs SET status = 'queued' WHERE job_id = 'job_1'",
                    &[],
                )
                .await
                .is_err()
        );
        client
            .execute(
                "UPDATE compute_jobs SET status = 'queued', assignment_lease_id = NULL WHERE job_id = 'job_1'",
                &[],
            )
            .await
            .unwrap();
        drop(client);
        db.drop_schema_for_test().await.unwrap();
    }

    #[test]
    fn customer_accounts_reservations_migration_declares_customer_registry() {
        let sql = MIGRATIONS[15].sql;
        for table in [
            "organizations",
            "organization_users",
            "projects",
            "project_quotas",
            "customer_api_keys",
            "customer_credit_ledger_entries",
            "marketplace_reservations",
            "customer_audit_events",
        ] {
            assert!(sql.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")));
        }
        assert!(sql.contains("idx_marketplace_reservations_active_listing"));
        assert!(sql.contains("prevent_customer_credit_ledger_mutation"));
    }
}
