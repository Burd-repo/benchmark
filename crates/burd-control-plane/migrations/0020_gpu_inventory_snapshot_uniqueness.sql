ALTER TABLE device_gpu_inventory
    DROP CONSTRAINT IF EXISTS device_gpu_inventory_inventory_hash_key;

CREATE UNIQUE INDEX IF NOT EXISTS idx_device_gpu_inventory_snapshot_gpu
    ON device_gpu_inventory(inventory_hash, gpu_index);

CREATE INDEX IF NOT EXISTS idx_device_gpu_inventory_hash
    ON device_gpu_inventory(inventory_hash);