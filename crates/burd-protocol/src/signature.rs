use crate::report::ReportSignature;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const KEY_ALGORITHM: &str = "ed25519";

#[derive(Debug, Clone)]
pub struct KeyMaterial {
    pub secret_key_base64: String,
    pub public_key_base64: String,
}

pub fn generate_keypair() -> Result<KeyMaterial, String> {
    let mut secret = [0u8; 32];
    getrandom::getrandom(&mut secret)
        .map_err(|error| format!("failed to generate key: {error}"))?;
    let signing_key = SigningKey::from_bytes(&secret);
    Ok(KeyMaterial {
        secret_key_base64: encode_base64(&secret),
        public_key_base64: encode_base64(signing_key.verifying_key().as_bytes()),
    })
}

pub fn sign_message(secret_key_base64: &str, message: &[u8]) -> Result<String, String> {
    let secret = decode_fixed::<32>(secret_key_base64, "secret key")?;
    let signing_key = SigningKey::from_bytes(&secret);
    let signature = signing_key.sign(message);
    Ok(encode_base64(&signature.to_bytes()))
}

pub fn verify_message(
    public_key_base64: &str,
    message: &[u8],
    signature_base64: &str,
) -> Result<bool, String> {
    let public = decode_fixed::<32>(public_key_base64, "public key")?;
    let signature = STANDARD
        .decode(signature_base64)
        .map_err(|error| format!("invalid signature base64: {error}"))?;
    let signature = Signature::try_from(signature.as_slice())
        .map_err(|error| format!("invalid signature bytes: {error}"))?;
    let verifying_key = VerifyingKey::from_bytes(&public)
        .map_err(|error| format!("invalid public key: {error}"))?;
    Ok(verifying_key.verify(message, &signature).is_ok())
}

pub fn canonical_json_value(value: &Value) -> Result<String, String> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_string(value)
                .map_err(|error| format!("failed to serialize JSON: {error}"))
        }
        Value::Array(items) => {
            let mut out = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json_value(item)?);
            }
            out.push(']');
            Ok(out)
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(
                    &serde_json::to_string(key)
                        .map_err(|error| format!("failed to serialize JSON key: {error}"))?,
                );
                out.push(':');
                let item = map
                    .get(*key)
                    .ok_or_else(|| format!("missing JSON key during canonicalization: {key}"))?;
                out.push_str(&canonical_json_value(item)?);
            }
            out.push('}');
            Ok(out)
        }
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String, String> {
    let value =
        serde_json::to_value(value).map_err(|error| format!("failed to convert JSON: {error}"))?;
    canonical_json_value(&value)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

pub fn hash_canonical<T: Serialize>(value: &T) -> Result<String, String> {
    let canonical = canonical_json(value)?;
    Ok(sha256_hex(canonical.as_bytes()))
}

pub fn placeholder_signature(
    machine_id: Option<&str>,
    challenge_id: Option<&str>,
) -> ReportSignature {
    let machine = machine_id.unwrap_or("unknown-machine");
    let challenge = challenge_id.unwrap_or("no-challenge");
    ReportSignature {
        algorithm: "placeholder-ed25519".to_string(),
        value: format!("placeholder-signature:{machine}:{challenge}"),
        status: "mocked".to_string(),
    }
}

pub fn encode_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode_base64(value: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(value)
        .map_err(|error| format!("invalid base64: {error}"))
}

fn decode_fixed<const N: usize>(value: &str, label: &str) -> Result<[u8; N], String> {
    let bytes = decode_base64(value)?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| format!("{label} must be {N} bytes, got {}", bytes.len()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_object_keys() {
        let value = serde_json::json!({"b": 2, "a": {"d": 4, "c": 3}});
        assert_eq!(
            canonical_json_value(&value).unwrap(),
            r#"{"a":{"c":3,"d":4},"b":2}"#
        );
    }

    #[test]
    fn ed25519_signature_roundtrip() {
        let keys = generate_keypair().unwrap();
        let message = b"burd-report";
        let signature = sign_message(&keys.secret_key_base64, message).unwrap();
        assert!(
            verify_message(&keys.public_key_base64, message, &signature).unwrap(),
            "signature should verify"
        );
    }
}
