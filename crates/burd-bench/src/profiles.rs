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
        profile(ProfileDefinition {
            id: "profile_8gb",
            min_vram_gb: 8.0,
            suggested_model: "llama3.2:1b",
            suggested_runtime: "ollama",
            default_runs: 3,
            default_context: 4096,
            workloads: &["LLM leve", "embeddings", "Whisper", "agentes simples"],
            min_avg_tps: 20.0,
            notes: &["Perfil de entrada para validacao de GPU pequena."],
        }),
        profile(ProfileDefinition {
            id: "profile_12gb",
            min_vram_gb: 12.0,
            suggested_model: "llama3.2:3b",
            suggested_runtime: "ollama",
            default_runs: 3,
            default_context: 8192,
            workloads: &[
                "LLMs pequenos quantizados",
                "Stable Diffusion basico",
                "Whisper",
                "embeddings",
                "bots/agentes leves",
            ],
            min_avg_tps: 18.0,
            notes: &["Boa base para workloads leves e medios quantizados."],
        }),
        profile(ProfileDefinition {
            id: "profile_16gb",
            min_vram_gb: 16.0,
            suggested_model: "qwen2.5:7b",
            suggested_runtime: "ollama",
            default_runs: 3,
            default_context: 8192,
            workloads: &[
                "LLMs medios quantizados",
                "ComfyUI basico",
                "batch inference pequeno",
            ],
            min_avg_tps: 14.0,
            notes: &["Perfil intermediario para modelos 7B quantizados."],
        }),
        profile(ProfileDefinition {
            id: "profile_24gb",
            min_vram_gb: 24.0,
            suggested_model: "qwen2.5:14b",
            suggested_runtime: "ollama",
            default_runs: 3,
            default_context: 8192,
            workloads: &[
                "SDXL",
                "ComfyUI",
                "LLMs medios quantizados",
                "agentes",
                "inferencia rapida",
                "batch pequeno",
            ],
            min_avg_tps: 10.0,
            notes: &["Perfil forte para GPUs como RTX 3090 e RTX 4090."],
        }),
        profile(ProfileDefinition {
            id: "profile_48gb",
            min_vram_gb: 48.0,
            suggested_model: "qwen2.5:32b",
            suggested_runtime: "vllm",
            default_runs: 3,
            default_context: 8192,
            workloads: &[
                "LLMs maiores",
                "batch inference",
                "ComfyUI avancado",
                "workloads enterprise leves",
            ],
            min_avg_tps: 8.0,
            notes: &["Perfil multiusuario ou de alto volume."],
        }),
        profile(ProfileDefinition {
            id: "profile_80gb",
            min_vram_gb: 80.0,
            suggested_model: "Qwen/Qwen2.5-72B-Instruct",
            suggested_runtime: "vllm",
            default_runs: 3,
            default_context: 8192,
            workloads: &[
                "fine-tuning",
                "batch inference",
                "modelos maiores",
                "workloads enterprise",
            ],
            min_avg_tps: 6.0,
            notes: &["Perfil datacenter para A100/H100 e workloads enterprise."],
        }),
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

struct ProfileDefinition<'a> {
    id: &'a str,
    min_vram_gb: f64,
    suggested_model: &'a str,
    suggested_runtime: &'a str,
    default_runs: usize,
    default_context: u32,
    workloads: &'a [&'a str],
    min_avg_tps: f64,
    notes: &'a [&'a str],
}

fn profile(definition: ProfileDefinition<'_>) -> BenchmarkProfile {
    BenchmarkProfile {
        id: definition.id.to_string(),
        min_vram_gb: definition.min_vram_gb,
        suggested_model: definition.suggested_model.to_string(),
        suggested_runtime: definition.suggested_runtime.to_string(),
        default_runs: definition.default_runs,
        default_context: definition.default_context,
        recommended_workloads: definition
            .workloads
            .iter()
            .map(|item| item.to_string())
            .collect(),
        min_avg_tps: definition.min_avg_tps,
        notes: definition
            .notes
            .iter()
            .map(|item| item.to_string())
            .collect(),
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
