use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const FULL_REPORT_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const SIGNED_REPORT_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const CHALLENGE_TTL_SECONDS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFreshness {
    pub issued_at: String,
    pub expires_at: String,
    pub is_expired: bool,
    pub age_seconds: u64,
    pub ttl_seconds: u64,
}

pub fn evidence_freshness(issued_at: &str, ttl_seconds: u64) -> Result<EvidenceFreshness, String> {
    evidence_freshness_at(issued_at, ttl_seconds, Utc::now())
}

pub fn evidence_freshness_at(
    issued_at: &str,
    ttl_seconds: u64,
    now: DateTime<Utc>,
) -> Result<EvidenceFreshness, String> {
    let issued = parse_rfc3339(issued_at, "issued_at")?;
    let expires = issued + Duration::seconds(ttl_seconds as i64);
    Ok(build_freshness(issued, expires, now))
}

pub fn evidence_freshness_from_window(
    issued_at: &str,
    expires_at: &str,
) -> Result<EvidenceFreshness, String> {
    evidence_freshness_from_window_at(issued_at, expires_at, Utc::now())
}

pub fn evidence_freshness_from_window_at(
    issued_at: &str,
    expires_at: &str,
    now: DateTime<Utc>,
) -> Result<EvidenceFreshness, String> {
    let issued = parse_rfc3339(issued_at, "issued_at")?;
    let expires = parse_rfc3339(expires_at, "expires_at")?;
    if expires < issued {
        return Err("expires_at is before issued_at".to_string());
    }
    Ok(build_freshness(issued, expires, now))
}

fn parse_rfc3339(value: &str, field: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("invalid evidence {field}: {error}"))
}

fn build_freshness(
    issued: DateTime<Utc>,
    expires: DateTime<Utc>,
    now: DateTime<Utc>,
) -> EvidenceFreshness {
    EvidenceFreshness {
        issued_at: issued.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
        is_expired: now > expires,
        age_seconds: now.signed_duration_since(issued).num_seconds().max(0) as u64,
        ttl_seconds: expires.signed_duration_since(issued).num_seconds().max(0) as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_marks_valid_and_expired_windows() {
        let issued = DateTime::parse_from_rfc3339("2026-06-08T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let valid =
            evidence_freshness_at("2026-06-08T00:00:00Z", 86_400, issued + Duration::hours(12))
                .unwrap();
        assert!(!valid.is_expired);
        assert_eq!(valid.age_seconds, 43_200);
        assert_eq!(valid.ttl_seconds, 86_400);

        let expired =
            evidence_freshness_at("2026-06-08T00:00:00Z", 86_400, issued + Duration::hours(25))
                .unwrap();
        assert!(expired.is_expired);
    }
}
