ALTER TABLE customer_workloads
    ADD COLUMN IF NOT EXISTS reservation_id TEXT
        REFERENCES marketplace_reservations(reservation_id);

ALTER TABLE compute_placements
    ADD COLUMN IF NOT EXISTS reservation_id TEXT
        REFERENCES marketplace_reservations(reservation_id);

ALTER TABLE compute_jobs
    ADD COLUMN IF NOT EXISTS reservation_id TEXT
        REFERENCES marketplace_reservations(reservation_id);

ALTER TABLE marketplace_reservations
    DROP CONSTRAINT IF EXISTS marketplace_reservations_status_allowed;
ALTER TABLE marketplace_reservations
    ADD CONSTRAINT marketplace_reservations_status_allowed CHECK (
        status IN ('reserved', 'consumed', 'released', 'cancelled', 'expired')
    );

CREATE UNIQUE INDEX IF NOT EXISTS idx_customer_workloads_reservation
    ON customer_workloads(reservation_id)
    WHERE reservation_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_customer_workloads_reservation_workload
    ON customer_workloads(reservation_id, workload_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_compute_placements_reservation
    ON compute_placements(reservation_id)
    WHERE reservation_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_compute_jobs_reservation
    ON compute_jobs(reservation_id)
    WHERE reservation_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_marketplace_reservations_consumption
    ON marketplace_reservations(project_id, status, expires_at, reservation_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_marketplace_reservations_project_binding
    ON marketplace_reservations(reservation_id, organization_id, project_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_marketplace_reservations_supply_binding
    ON marketplace_reservations(
        reservation_id, listing_id, provider_id, device_id, session_id, gpu_uuid
    );

ALTER TABLE customer_workloads
    ADD CONSTRAINT customer_workloads_reservation_binding_fk
    FOREIGN KEY (reservation_id, organization_id, project_id)
    REFERENCES marketplace_reservations(reservation_id, organization_id, project_id);
ALTER TABLE compute_placements
    ADD CONSTRAINT compute_placements_reservation_binding_fk
    FOREIGN KEY (reservation_id, listing_id, provider_id, device_id, session_id, gpu_uuid)
    REFERENCES marketplace_reservations(
        reservation_id, listing_id, provider_id, device_id, session_id, gpu_uuid
    );
ALTER TABLE compute_jobs
    ADD CONSTRAINT compute_jobs_reservation_workload_binding_fk
    FOREIGN KEY (reservation_id, workload_id)
    REFERENCES customer_workloads(reservation_id, workload_id);
