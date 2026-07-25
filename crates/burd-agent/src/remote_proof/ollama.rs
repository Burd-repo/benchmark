use burd_protocol::ProofCapabilityChallenge;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

const OLLAMA_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LlmProofResult {
    pub(super) tokens_per_second: f64,
    pub(super) ttft_ms: u64,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
    digest: String,
}

#[derive(Debug, Default, Deserialize)]
struct OllamaStreamResponse {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    eval_count: Option<u64>,
    #[serde(default)]
    eval_duration: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

pub(super) fn run_inference(
    challenge: &ProofCapabilityChallenge,
) -> Result<LlmProofResult, String> {
    let base_url =
        std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
    let model = find_model(&base_url, &challenge.model_artifact_hash)?;
    let url = format!("{}/api/generate", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "prompt": proof_prompt(challenge),
        "stream": true,
        "keep_alive": "5m",
        "options": {
            "num_predict": 128,
            "temperature": 0,
            "seed": stable_prompt_seed(&challenge.prompt_seed),
        }
    });
    let started = Instant::now();
    let response = ureq::post(&url)
        .config()
        .timeout_global(Some(OLLAMA_REQUEST_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .send_json(body)
        .map_err(|error| format!("Ollama proof request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Ollama proof request returned HTTP {}",
            response.status().as_u16()
        ));
    }

    let mut ttft_ms = None;
    let mut final_response = None;
    let reader = BufReader::new(response.into_body().into_reader());
    for line in reader.lines() {
        let line = line.map_err(|error| format!("Ollama proof stream read failed: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let chunk: OllamaStreamResponse = serde_json::from_str(&line)
            .map_err(|error| format!("Ollama proof stream returned invalid JSON: {error}"))?;
        if let Some(error) = chunk.error.as_deref() {
            return Err(format!("Ollama rejected proof inference: {error}"));
        }
        if ttft_ms.is_none() && !chunk.response.is_empty() {
            ttft_ms = Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
        }
        if chunk.done {
            final_response = Some(chunk);
            break;
        }
    }

    let final_response = final_response
        .ok_or_else(|| "Ollama proof stream ended without final metrics".to_string())?;
    let eval_count = final_response
        .eval_count
        .filter(|count| *count > 0)
        .ok_or_else(|| "Ollama proof response did not report generated tokens".to_string())?;
    let eval_duration = final_response
        .eval_duration
        .filter(|duration| *duration > 0)
        .ok_or_else(|| "Ollama proof response did not report evaluation duration".to_string())?;
    let tokens_per_second = eval_count as f64 / (eval_duration as f64 / 1_000_000_000.0);
    if !tokens_per_second.is_finite() || tokens_per_second <= 0.0 {
        return Err("Ollama proof tokens per second is invalid".to_string());
    }
    let ttft_ms = ttft_ms.ok_or_else(|| "Ollama proof produced no output token".to_string())?;
    if find_model(&base_url, &challenge.model_artifact_hash)? != model {
        return Err("Ollama model tag changed during proof execution".to_string());
    }
    Ok(LlmProofResult {
        tokens_per_second,
        ttft_ms,
    })
}

fn find_model(base_url: &str, required_digest: &str) -> Result<String, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let response = ureq::get(&url)
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .call()
        .map_err(|error| format!("Ollama model inventory request failed: {error}"))?;
    let mut tags: OllamaTagsResponse = response
        .into_body()
        .read_json()
        .map_err(|error| format!("Ollama model inventory returned invalid JSON: {error}"))?;
    tags.models
        .sort_by(|left, right| left.name.cmp(&right.name));
    tags.models
        .into_iter()
        .find(|model| model.digest.eq_ignore_ascii_case(required_digest))
        .map(|model| model.name)
        .ok_or_else(|| {
            format!("no local Ollama model matches required artifact digest {required_digest}")
        })
}

fn proof_prompt(challenge: &ProofCapabilityChallenge) -> String {
    format!(
        "Burd Proof of Capability challenge. Nonce: {}. Prompt seed: {}. Explain in 100 to 120 words how a deterministic hash chain prevents replay. Do not omit either identifier.",
        challenge.nonce, challenge.prompt_seed
    )
}

fn stable_prompt_seed(value: &str) -> i32 {
    value.as_bytes().iter().fold(0x811c9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    }) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use burd_protocol::PROOF_CHALLENGE_SCHEMA_VERSION;
    use chrono::Utc;

    #[test]
    fn proof_prompt_binds_nonce_and_seed() {
        let challenge = challenge();
        let prompt = proof_prompt(&challenge);
        assert!(prompt.contains(&challenge.nonce));
        assert!(prompt.contains(&challenge.prompt_seed));
        assert_eq!(stable_prompt_seed("seed_1"), stable_prompt_seed("seed_1"));
        assert_ne!(stable_prompt_seed("seed_1"), stable_prompt_seed("seed_2"));
    }

    fn challenge() -> ProofCapabilityChallenge {
        let issued_at = Utc::now();
        ProofCapabilityChallenge {
            schema_version: PROOF_CHALLENGE_SCHEMA_VERSION.to_string(),
            challenge_id: "challenge_1".to_string(),
            nonce: "nonce_1".to_string(),
            provider_id: "provider_1".to_string(),
            device_id: "device_1".to_string(),
            session_id: "session_1".to_string(),
            profile_version: "poc-cuda-llm-v1".to_string(),
            required_fingerprint: "sha256:fingerprint".to_string(),
            required_gpu_uuid: None,
            required_backend: "cuda".to_string(),
            model_artifact_hash: "sha256:model".to_string(),
            prompt_seed: "seed_1".to_string(),
            required_proofs: vec!["llm_short_inference".to_string()],
            min_tokens_per_second: 1.0,
            max_ttft_ms: 5_000,
            issued_at: issued_at.to_rfc3339(),
            expires_at: (issued_at + chrono::Duration::minutes(5)).to_rfc3339(),
        }
    }
}
