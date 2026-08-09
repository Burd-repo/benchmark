ALTER TABLE compute_jobs
    ADD COLUMN IF NOT EXISTS assignment_lease_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_job_leases_lease_job
    ON job_leases(lease_id, job_id);

-- Existing active jobs predate the explicit assignment binding. Prefer the sole active lease;
-- if authority was already lost, retain the most recently offered lease so acceptance can
-- fail closed without confusing it with an older acknowledgement.
WITH ranked_assignment_leases AS (
    SELECT
        job.job_id,
        lease.lease_id,
        ROW_NUMBER() OVER (
            PARTITION BY job.job_id
            ORDER BY
                CASE
                    WHEN lease.status IN ('offered', 'accepted', 'provisioning', 'active') THEN 0
                    ELSE 1
                END,
                lease.offered_at DESC,
                lease.lease_id DESC
        ) AS assignment_rank
    FROM compute_jobs AS job
    JOIN job_leases AS lease ON lease.job_id = job.job_id
    WHERE job.status IN ('assigned', 'accepted', 'provisioning', 'running', 'uploading')
)
UPDATE compute_jobs AS job
SET assignment_lease_id = ranked.lease_id
FROM ranked_assignment_leases AS ranked
WHERE ranked.job_id = job.job_id
  AND ranked.assignment_rank = 1
  AND job.assignment_lease_id IS NULL;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM compute_jobs
        WHERE status IN ('assigned', 'accepted', 'provisioning', 'running', 'uploading')
          AND assignment_lease_id IS NULL
    ) THEN
        RAISE EXCEPTION 'active compute job is missing an assignment lease';
    END IF;
END;
$$;

ALTER TABLE compute_jobs
    ADD CONSTRAINT compute_jobs_assignment_lease_binding_fk
    FOREIGN KEY (assignment_lease_id, job_id)
    REFERENCES job_leases(lease_id, job_id)
    DEFERRABLE INITIALLY DEFERRED;

ALTER TABLE compute_jobs
    ADD CONSTRAINT compute_jobs_active_assignment_lease_check
    CHECK (
        status NOT IN ('assigned', 'accepted', 'provisioning', 'running', 'uploading')
        OR assignment_lease_id IS NOT NULL
    ),
    ADD CONSTRAINT compute_jobs_queued_assignment_lease_check
    CHECK (status <> 'queued' OR assignment_lease_id IS NULL);

CREATE INDEX IF NOT EXISTS idx_compute_jobs_assignment_lease
    ON compute_jobs(assignment_lease_id)
    WHERE assignment_lease_id IS NOT NULL;
