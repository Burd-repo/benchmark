use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub challenge_id: String,
    pub nonce: String,
    pub benchmark_profile: String,
    pub expires_at: String,
    pub backend_url: String,
    pub required_tests: Vec<RequiredTest>,
    pub issued_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredTest {
    pub name: String,
    pub required: bool,
}

pub fn mock_challenge(profile: &str) -> Challenge {
    let issued = Utc::now();
    Challenge {
        challenge_id: format!("challenge-{}", Uuid::new_v4()),
        nonce: Uuid::new_v4().to_string(),
        benchmark_profile: profile.to_string(),
        expires_at: (issued + Duration::minutes(30)).to_rfc3339(),
        backend_url: "https://api.burd.cloud".to_string(),
        required_tests: vec![
            RequiredTest {
                name: "system".to_string(),
                required: true,
            },
            RequiredTest {
                name: "fit".to_string(),
                required: true,
            },
            RequiredTest {
                name: "llm_benchmark".to_string(),
                required: true,
            },
            RequiredTest {
                name: "stability".to_string(),
                required: true,
            },
            RequiredTest {
                name: "network".to_string(),
                required: false,
            },
            RequiredTest {
                name: "disk".to_string(),
                required: false,
            },
        ],
        issued_at: issued.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_serializes() {
        let challenge = mock_challenge("profile_24gb");
        let json = serde_json::to_string(&challenge).unwrap();
        assert!(json.contains("challenge_id"));
        assert!(json.contains("profile_24gb"));
    }
}
