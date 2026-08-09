ALTER TABLE compute_jobs
    ADD COLUMN IF NOT EXISTS scheduler_last_evaluated_at TEXT;

CREATE INDEX IF NOT EXISTS idx_compute_jobs_scheduler_fairness
    ON compute_jobs(status, COALESCE(scheduler_last_evaluated_at, created_at), created_at, job_id);
