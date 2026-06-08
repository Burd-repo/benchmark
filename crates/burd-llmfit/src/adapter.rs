use llmfit_core::fit::{
    FitLevel, InferenceRuntime, ModelFit, RunMode, SortColumn, backend_compatible,
    rank_models_by_fit_opts_col,
};
use llmfit_core::hardware::SystemSpecs;
use llmfit_core::models::{ModelDatabase, UseCase};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurdFitModel {
    pub name: String,
    pub provider: String,
    pub parameter_count: String,
    pub fit_level: String,
    pub run_mode: String,
    pub best_quantization: String,
    pub memory_estimated_gb: f64,
    pub memory_available_gb: f64,
    pub memory_usage_pct: f64,
    pub estimated_tps: f64,
    pub effective_context: u32,
    pub category: String,
    pub workloads: Vec<String>,
    pub runtime: String,
    pub score: f64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitReport {
    pub models: Vec<BurdFitModel>,
    pub recommended_workloads: Vec<String>,
    pub not_recommended_workloads: Vec<String>,
    pub provider_capability_summary: String,
    pub total_models_analyzed: usize,
    pub runnable_models: usize,
    pub source: String,
}

pub fn build_fit_report(specs: &SystemSpecs, limit: Option<usize>) -> FitReport {
    let db = ModelDatabase::new();
    let all = db.get_all_models();
    let mut fits: Vec<ModelFit> = all
        .iter()
        .filter(|model| backend_compatible(model, specs))
        .map(|model| ModelFit::analyze(model, specs))
        .collect();

    fits = rank_models_by_fit_opts_col(fits, false, SortColumn::Score);
    let total_models_analyzed = fits.len();
    let runnable_models = fits
        .iter()
        .filter(|fit| fit.fit_level != FitLevel::TooTight)
        .count();

    let selected = if let Some(limit) = limit {
        fits.into_iter().take(limit).collect()
    } else {
        fits
    };

    let models: Vec<BurdFitModel> = selected.iter().map(fit_to_burd).collect();
    let vram = specs.total_gpu_vram_gb.or(specs.gpu_vram_gb).unwrap_or(0.0);
    let (recommended, not_recommended) = workload_summary_from_vram(vram, specs.has_gpu);

    FitReport {
        provider_capability_summary: capability_summary(specs, &models),
        models,
        recommended_workloads: recommended,
        not_recommended_workloads: not_recommended,
        total_models_analyzed,
        runnable_models,
        source: "llmfit-core adapter".to_string(),
    }
}

fn fit_to_burd(fit: &ModelFit) -> BurdFitModel {
    BurdFitModel {
        name: fit.model.name.clone(),
        provider: fit.model.provider.clone(),
        parameter_count: fit.model.parameter_count.clone(),
        fit_level: fit_level_label(fit.fit_level).to_string(),
        run_mode: run_mode_label(fit.run_mode).to_string(),
        best_quantization: fit.best_quant.clone(),
        memory_estimated_gb: round2(fit.memory_required_gb),
        memory_available_gb: round2(fit.memory_available_gb),
        memory_usage_pct: round1(fit.utilization_pct),
        estimated_tps: round1(fit.estimated_tps),
        effective_context: fit.effective_context_length,
        category: use_case_label(fit.use_case).to_string(),
        workloads: workloads_for_use_case(fit.use_case),
        runtime: runtime_label(fit.runtime).to_string(),
        score: round1(fit.score),
        notes: fit.notes.clone(),
    }
}

pub fn fit_level_label(level: FitLevel) -> &'static str {
    match level {
        FitLevel::Perfect => "Perfect",
        FitLevel::Good => "Good",
        FitLevel::Marginal => "Marginal",
        FitLevel::TooTight => "Too Tight",
    }
}

pub fn run_mode_label(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Gpu => "GPU",
        RunMode::CpuOffload => "CPU+GPU",
        RunMode::CpuOnly => "CPU",
        RunMode::MoeOffload => "MoE Offload",
        RunMode::TensorParallel => "Tensor Parallel",
    }
}

pub fn runtime_label(runtime: InferenceRuntime) -> &'static str {
    match runtime {
        InferenceRuntime::LlamaCpp => "llama.cpp/Ollama",
        InferenceRuntime::Mlx => "MLX",
        InferenceRuntime::Vllm => "vLLM",
    }
}

fn use_case_label(use_case: UseCase) -> &'static str {
    match use_case {
        UseCase::General => "General",
        UseCase::Coding => "Coding",
        UseCase::Reasoning => "Reasoning",
        UseCase::Chat => "Chat",
        UseCase::Multimodal => "Multimodal",
        UseCase::Embedding => "Embedding",
    }
}

fn workloads_for_use_case(use_case: UseCase) -> Vec<String> {
    match use_case {
        UseCase::Embedding => vec!["embeddings".to_string(), "semantic search".to_string()],
        UseCase::Multimodal => vec![
            "vision-language inference".to_string(),
            "image understanding".to_string(),
        ],
        UseCase::Coding => vec!["coding agents".to_string(), "dev automation".to_string()],
        UseCase::Reasoning => vec!["reasoning agents".to_string(), "analysis".to_string()],
        UseCase::Chat => vec!["chatbots".to_string(), "support agents".to_string()],
        UseCase::General => vec!["LLM inference".to_string(), "agents".to_string()],
    }
}

pub fn workload_summary_from_vram(vram_gb: f64, has_gpu: bool) -> (Vec<String>, Vec<String>) {
    if !has_gpu {
        return (
            vec!["CPU fallback".to_string(), "small embeddings".to_string()],
            vec![
                "LLM medio quantizado".to_string(),
                "Stable Diffusion".to_string(),
                "SDXL".to_string(),
                "fine-tuning".to_string(),
            ],
        );
    }

    if vram_gb >= 80.0 {
        (
            vec![
                "fine-tuning".to_string(),
                "batch inference".to_string(),
                "modelos maiores".to_string(),
                "workloads enterprise".to_string(),
            ],
            vec![],
        )
    } else if vram_gb >= 48.0 {
        (
            vec![
                "LLM grande quantizado".to_string(),
                "batch inference".to_string(),
                "ComfyUI avancado".to_string(),
                "workloads enterprise leves".to_string(),
            ],
            vec!["fine-tuning pesado".to_string()],
        )
    } else if vram_gb >= 24.0 {
        (
            vec![
                "SDXL".to_string(),
                "ComfyUI".to_string(),
                "LLM medio quantizado".to_string(),
                "agentes".to_string(),
                "batch inference pequeno".to_string(),
            ],
            vec![
                "fine-tuning pesado".to_string(),
                "modelos enterprise grandes".to_string(),
            ],
        )
    } else if vram_gb >= 12.0 {
        (
            vec![
                "LLM leve".to_string(),
                "LLM medio quantizado".to_string(),
                "embeddings".to_string(),
                "Whisper".to_string(),
                "Stable Diffusion basico".to_string(),
                "agentes leves".to_string(),
            ],
            vec![
                "SDXL pesado".to_string(),
                "fine-tuning".to_string(),
                "workloads enterprise".to_string(),
            ],
        )
    } else if vram_gb >= 8.0 {
        (
            vec![
                "LLM leve".to_string(),
                "embeddings".to_string(),
                "Whisper".to_string(),
                "agentes simples".to_string(),
            ],
            vec![
                "LLM medio".to_string(),
                "SDXL".to_string(),
                "fine-tuning".to_string(),
            ],
        )
    } else {
        (
            vec![
                "embeddings pequenos".to_string(),
                "CPU fallback".to_string(),
            ],
            vec![
                "LLM medio".to_string(),
                "Stable Diffusion".to_string(),
                "SDXL".to_string(),
                "fine-tuning".to_string(),
            ],
        )
    }
}

fn capability_summary(specs: &SystemSpecs, models: &[BurdFitModel]) -> String {
    let gpu = specs
        .gpu_name
        .clone()
        .unwrap_or_else(|| "CPU fallback".to_string());
    let vram = specs
        .total_gpu_vram_gb
        .or(specs.gpu_vram_gb)
        .map(|v| format!("{v:.1} GB VRAM"))
        .unwrap_or_else(|| "VRAM desconhecida".to_string());
    let top = models
        .iter()
        .find(|model| model.fit_level != "Too Tight")
        .map(|model| format!("top fit: {} ({})", model.name, model.fit_level))
        .unwrap_or_else(|| "nenhum modelo recomendado com confianca".to_string());

    format!(
        "{} via {} com {}; {}.",
        gpu,
        specs.backend.label(),
        vram,
        top
    )
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vram_workloads_scale_up() {
        let (recommended, not_recommended) = workload_summary_from_vram(24.0, true);
        assert!(recommended.contains(&"SDXL".to_string()));
        assert!(not_recommended.contains(&"fine-tuning pesado".to_string()));
    }
}
