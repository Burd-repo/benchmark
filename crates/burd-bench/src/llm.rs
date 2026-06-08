use crate::profiles::profile_for_vram;
use llmfit_core::bench::{self, BenchResult, BenchTarget};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmBenchmarkOptions {
    pub provider: String,
    pub url: Option<String>,
    pub model: Option<String>,
    pub runs: usize,
    pub profile: Option<String>,
    pub detected_vram_gb: f64,
}

impl Default for LlmBenchmarkOptions {
    fn default() -> Self {
        Self {
            provider: "ollama".to_string(),
            url: None,
            model: None,
            runs: 3,
            profile: None,
            detected_vram_gb: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRunReport {
    pub ttft_ms: Option<f64>,
    pub tps: f64,
    pub latency_ms: f64,
    pub prompt_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmBenchmarkReport {
    pub provider: String,
    pub model: String,
    pub runtime: String,
    pub runs: usize,
    pub avg_tps: f64,
    pub min_tps: f64,
    pub max_tps: f64,
    pub stddev_tps: f64,
    pub avg_ttft_ms: Option<f64>,
    pub avg_latency_ms: f64,
    pub prompt_tokens_avg: f64,
    pub output_tokens_avg: f64,
    pub total_tokens_avg: f64,
    pub total_duration_ms: f64,
    pub passed: bool,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub run_details: Vec<LlmRunReport>,
}

pub fn run_llm_benchmark(options: LlmBenchmarkOptions) -> LlmBenchmarkReport {
    let runs = options.runs.max(1);
    let profile = profile_for_vram(options.detected_vram_gb);
    let provider = options.provider.to_lowercase();
    let model = options
        .model
        .clone()
        .unwrap_or_else(|| profile.suggested_model.clone());

    let result = match provider.as_str() {
        "ollama" => {
            let url = options
                .url
                .clone()
                .or_else(|| std::env::var("OLLAMA_HOST").ok())
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            bench::bench_ollama(&url, &model, runs, &|_, _| {})
        }
        "vllm" => {
            let url = options
                .url
                .clone()
                .unwrap_or_else(|| "http://localhost:8000".to_string());
            bench::bench_openai_compat(&url, &model, "vllm", runs, &|_, _| {})
        }
        "mlx" => {
            let url = options
                .url
                .clone()
                .unwrap_or_else(|| "http://localhost:8080".to_string());
            bench::bench_openai_compat(&url, &model, "mlx", runs, &|_, _| {})
        }
        "auto" => match bench::auto_detect_target(options.model.as_deref()) {
            Ok(BenchTarget::Ollama { url, model }) => {
                bench::bench_ollama(&url, &model, runs, &|_, _| {})
            }
            Ok(BenchTarget::VLlm { url, model }) => {
                bench::bench_openai_compat(&url, &model, "vllm", runs, &|_, _| {})
            }
            Ok(BenchTarget::Mlx { url, model }) => {
                bench::bench_openai_compat(&url, &model, "mlx", runs, &|_, _| {})
            }
            Err(error) => Err(error),
        },
        other => Err(format!(
            "unsupported provider '{other}', expected ollama, vllm, mlx or auto"
        )),
    };

    match result {
        Ok(result) => report_from_llmfit(result, profile.min_avg_tps),
        Err(error) => failed_report(provider, model, runs, error),
    }
}

fn report_from_llmfit(result: BenchResult, min_avg_tps: f64) -> LlmBenchmarkReport {
    let run_details: Vec<LlmRunReport> = result
        .runs
        .iter()
        .map(|run| LlmRunReport {
            ttft_ms: run.ttft_ms.map(round1),
            tps: round2(run.tps),
            latency_ms: round1(run.total_ms),
            prompt_tokens: run.prompt_tokens,
            output_tokens: run.output_tokens,
        })
        .collect();
    let tps_values: Vec<f64> = result.runs.iter().map(|run| run.tps).collect();
    let stddev = stddev(&tps_values);
    let prompt_tokens_avg = average_u32(result.runs.iter().map(|run| run.prompt_tokens));
    let output_tokens_avg = average_u32(result.runs.iter().map(|run| run.output_tokens));

    let mut warnings = Vec::new();
    if result.summary.num_runs < 3 {
        warnings.push("low reliability: fewer than 3 measured runs".to_string());
    }
    if result.summary.avg_tps > 0.0 && stddev / result.summary.avg_tps > 0.25 {
        warnings.push("high TPS variance between runs".to_string());
    }
    if result.summary.avg_tps < min_avg_tps {
        warnings.push(format!(
            "average TPS below profile threshold ({:.1} < {:.1})",
            result.summary.avg_tps, min_avg_tps
        ));
    }

    LlmBenchmarkReport {
        provider: result.provider.clone(),
        model: result.model.clone(),
        runtime: result.provider,
        runs: result.summary.num_runs,
        avg_tps: round2(result.summary.avg_tps),
        min_tps: round2(result.summary.min_tps),
        max_tps: round2(result.summary.max_tps),
        stddev_tps: round2(stddev),
        avg_ttft_ms: result.summary.avg_ttft_ms.map(round1),
        avg_latency_ms: round1(result.summary.avg_total_ms),
        prompt_tokens_avg: round1(prompt_tokens_avg),
        output_tokens_avg: round1(output_tokens_avg),
        total_tokens_avg: round1(prompt_tokens_avg + output_tokens_avg),
        total_duration_ms: round1(result.runs.iter().map(|run| run.total_ms).sum()),
        passed: result.summary.avg_tps > 0.0,
        warnings,
        errors: vec![],
        run_details,
    }
}

fn failed_report(
    provider: String,
    model: String,
    runs: usize,
    error: String,
) -> LlmBenchmarkReport {
    LlmBenchmarkReport {
        provider: provider.clone(),
        model,
        runtime: provider,
        runs,
        avg_tps: 0.0,
        min_tps: 0.0,
        max_tps: 0.0,
        stddev_tps: 0.0,
        avg_ttft_ms: None,
        avg_latency_ms: 0.0,
        prompt_tokens_avg: 0.0,
        output_tokens_avg: 0.0,
        total_tokens_avg: 0.0,
        total_duration_ms: 0.0,
        passed: false,
        warnings: vec!["real LLM benchmark skipped or failed".to_string()],
        errors: vec![error],
        run_details: vec![],
    }
}

fn average_u32(values: impl Iterator<Item = u32>) -> f64 {
    let values: Vec<u32> = values.collect();
    if values.is_empty() {
        return 0.0;
    }
    values.iter().map(|v| *v as f64).sum::<f64>() / values.len() as f64
}

fn stddev(values: &[f64]) -> f64 {
    if values.len() <= 1 {
        return 0.0;
    }
    let avg = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - avg;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
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
    fn failed_report_is_valid_json() {
        let report = failed_report(
            "ollama".to_string(),
            "llama3.2:1b".to_string(),
            3,
            "offline".to_string(),
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("offline"));
        assert!(!report.passed);
    }
}
