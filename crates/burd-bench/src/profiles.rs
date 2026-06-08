use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkProfile {
    pub id: String,
    pub min_vram_gb: f64,
    pub suggested_model: String,
    pub suggested_runtime: String,
    pub default_runs: usize,
    pub default_context: u32,
    pub recommended_workloads: Vec<String>,
    pub min_avg_tps: f64,
    pub notes: Vec<String>,
}

pub fn all_profiles() -> Vec<BenchmarkProfile> {
    vec![
        profile(
            "profile_8gb",
            8.0,
            "llama3.2:1b",
            "ollama",
            3,
            4096,
            &["LLM leve", "embeddings", "Whisper", "agentes simples"],
            20.0,
            &["Perfil de entrada para validacao de GPU pequena."],
        ),
        profile(
            "profile_12gb",
            12.0,
            "llama3.2:3b",
            "ollama",
            3,
            8192,
            &[
                "LLMs pequenos quantizados",
                "Stable Diffusion basico",
                "Whisper",
                "embeddings",
                "bots/agentes leves",
            ],
            18.0,
            &["Boa base para workloads leves e medios quantizados."],
        ),
        profile(
            "profile_16gb",
            16.0,
            "qwen2.5:7b",
            "ollama",
            3,
            8192,
            &[
                "LLMs medios quantizados",
                "ComfyUI basico",
                "batch inference pequeno",
            ],
            14.0,
            &["Perfil intermediario para modelos 7B quantizados."],
        ),
        profile(
            "profile_24gb",
            24.0,
            "qwen2.5:14b",
            "ollama",
            3,
            8192,
            &[
                "SDXL",
                "ComfyUI",
                "LLMs medios quantizados",
                "agentes",
                "inferencia rapida",
                "batch pequeno",
            ],
            10.0,
            &["Perfil forte para GPUs como RTX 3090 e RTX 4090."],
        ),
        profile(
            "profile_48gb",
            48.0,
            "qwen2.5:32b",
            "vllm",
            3,
            8192,
            &[
                "LLMs maiores",
                "batch inference",
                "ComfyUI avancado",
                "workloads enterprise leves",
            ],
            8.0,
            &["Perfil multiusuario ou de alto volume."],
        ),
        profile(
            "profile_80gb",
            80.0,
            "Qwen/Qwen2.5-72B-Instruct",
            "vllm",
            3,
            8192,
            &[
                "fine-tuning",
                "batch inference",
                "modelos maiores",
                "workloads enterprise",
            ],
            6.0,
            &["Perfil datacenter para A100/H100 e workloads enterprise."],
        ),
    ]
}

pub fn profile_for_vram(vram_gb: f64) -> BenchmarkProfile {
    let mut selected = all_profiles().remove(0);
    for profile in all_profiles() {
        if vram_gb >= profile.min_vram_gb {
            selected = profile;
        }
    }
    selected
}

fn profile(
    id: &str,
    min_vram_gb: f64,
    suggested_model: &str,
    suggested_runtime: &str,
    default_runs: usize,
    default_context: u32,
    workloads: &[&str],
    min_avg_tps: f64,
    notes: &[&str],
) -> BenchmarkProfile {
    BenchmarkProfile {
        id: id.to_string(),
        min_vram_gb,
        suggested_model: suggested_model.to_string(),
        suggested_runtime: suggested_runtime.to_string(),
        default_runs,
        default_context,
        recommended_workloads: workloads.iter().map(|item| item.to_string()).collect(),
        min_avg_tps,
        notes: notes.iter().map(|item| item.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_selection_uses_highest_matching_vram() {
        assert_eq!(profile_for_vram(12.0).id, "profile_12gb");
        assert_eq!(profile_for_vram(25.0).id, "profile_24gb");
        assert_eq!(profile_for_vram(90.0).id, "profile_80gb");
    }
}
