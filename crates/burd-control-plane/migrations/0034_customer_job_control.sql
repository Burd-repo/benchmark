CREATE INDEX IF NOT EXISTS idx_customer_workloads_project_job
    ON customer_workloads(project_id, job_id)
    WHERE job_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_job_events_customer_projection
    ON job_events(job_id, sequence ASC, event_id ASC);

ALTER TABLE customer_workloads
    DROP CONSTRAINT IF EXISTS customer_workloads_status_allowed;
ALTER TABLE customer_workloads
    ADD CONSTRAINT customer_workloads_status_allowed CHECK (
        status IN (
            'queued', 'placed', 'placement_failed',
            'succeeded', 'failed', 'cancelled'
        )
    );

ALTER TABLE customer_workloads
    DROP CONSTRAINT IF EXISTS customer_workloads_job_state;
ALTER TABLE customer_workloads
    ADD CONSTRAINT customer_workloads_job_state CHECK (
        (status IN ('placed', 'succeeded', 'failed', 'cancelled') AND job_id IS NOT NULL)
        OR (
            status IN ('queued', 'placement_failed')
            AND job_id IS NULL
        )
    );
