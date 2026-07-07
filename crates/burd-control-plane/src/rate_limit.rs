use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    buckets: Mutex<HashMap<String, RateBucket>>,
}

#[derive(Debug, Clone)]
struct RateBucket {
    window_start: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn per_minute(limit: u32) -> Self {
        Self {
            limit: limit.max(1),
            window: Duration::from_secs(60),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn check(&self, key: &str) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), u64> {
        let mut buckets = self.buckets.lock().expect("rate limit mutex poisoned");
        let bucket = buckets.entry(key.to_string()).or_insert(RateBucket {
            window_start: now,
            count: 0,
        });
        if now.duration_since(bucket.window_start) >= self.window {
            bucket.window_start = now;
            bucket.count = 0;
        }
        if bucket.count >= self.limit {
            let elapsed = now.duration_since(bucket.window_start);
            return Err(self.window.saturating_sub(elapsed).as_secs().max(1));
        }
        bucket.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_blocks_after_limit_and_recovers_after_window() {
        let limiter = RateLimiter::per_minute(1);
        let now = Instant::now();

        assert!(limiter.check_at("client", now).is_ok());
        assert!(
            limiter
                .check_at("client", now + Duration::from_secs(1))
                .is_err()
        );
        assert!(
            limiter
                .check_at("client", now + Duration::from_secs(61))
                .is_ok()
        );
    }
}
