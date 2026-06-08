use crate::report::ReportSignature;

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
