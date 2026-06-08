use crate::llm::{LlmBenchmarkOptions, run_llm_benchmark};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityBenchmarkReport {
    pub duration_seconds: u64,
    pub total_runs: u32,
    pub failed_runs: u32,
    pub initial_tps: Option<f64>,
    pub final_tps: Option<f64>,
    pub avg_tps: f64,
    pub performance_drop_pct: f64,
    pub timeouts: u32,
    pub oom_detected: bool,
    pub crashes_detected: bool,
    pub runtime_errors: Vec<String>,
    pub passed: bool,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
}

pub fn run_stability_benchmark(
    minutes: u64,
    mut options: LlmBenchmarkOptions,
) -> StabilityBenchmarkReport {
    options.runs = 1;
    let deadline = Instant::now() + Duration::from_secs(minutes.saturating_mul(60));
    let started = Instant::now();
    let mut tps_values = Vec::new();
    let mut failed_runs = 0;
    let mut errors = Vec::new();

    loop {
        let report = run_llm_benchmark(options.clone());
        if report.passed {
            tps_values.push(report.avg_tps);
        } else {
            failed_runs += 1;
            errors.extend(report.errors);
        }

        if minutes == 0 || Instant::now() >= deadline {
            break;
        }
    }

    evaluate_stability(
        &tps_values,
        failed_runs,
        false,
        started.elapsed().as_secs().max(minutes.saturating_mul(60)),
        errors,
    )
}

pub fn evaluate_stability(
    tps_values: &[f64],
    failed_runs: u32,
    critical_error: bool,
    duration_seconds: u64,
    runtime_errors: Vec<String>,
) -> StabilityBenchmarkReport {
    let total_runs = tps_values.len() as u32 + failed_runs;
    let initial_tps = tps_values.first().copied();
    let final_tps = tps_values.last().copied();
    let avg_tps = if tps_values.is_empty() {
        0.0
    } else {
        tps_values.iter().sum::<f64>() / tps_values.len() as f64
    };
    let performance_drop_pct = match (initial_tps, final_tps) {
        (Some(initial), Some(final_value)) if initial > 0.0 => {
            ((initial - final_value) / initial * 100.0).max(0.0)
        }
        _ => 0.0,
    };

    let oom_detected = runtime_errors.iter().any(|error| {
        error.to_lowercase().contains("oom") || error.to_lowercase().contains("out of memory")
    });
    let crashes_detected = runtime_errors.iter().any(|error| {
        error.to_lowercase().contains("crash") || error.to_lowercase().contains("connection reset")
    });

    let mut warnings = Vec::new();
    if performance_drop_pct > 25.0 {
        warnings.push("performance drop above 25%".to_string());
    }
    if failed_runs > 0 {
        warnings.push(format!("{failed_runs} failed benchmark runs"));
    }
    if total_runs == 0 {
        warnings.push("no stability runs completed".to_string());
    }

    let passed = total_runs > 0
        && !critical_error
        && !oom_detected
        && !crashes_detected
        && failed_runs <= 1
        && performance_drop_pct <= 25.0;

    StabilityBenchmarkReport {
        duration_seconds,
        total_runs,
        failed_runs,
        initial_tps: initial_tps.map(round2),
        final_tps: final_tps.map(round2),
        avg_tps: round2(avg_tps),
        performance_drop_pct: round1(performance_drop_pct),
        timeouts: runtime_errors
            .iter()
            .filter(|error| error.to_lowercase().contains("timeout"))
            .count() as u32,
        oom_detected,
        crashes_detected,
        runtime_errors,
        passed,
        warnings,
        notes: vec![
            "MVP stability benchmark reuses LLM benchmark loop.".to_string(),
            "Critical thermal and power telemetry can be added in a later backend-connected phase."
                .to_string(),
        ],
    }
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
    fn stability_fails_on_large_drop() {
        let report = evaluate_stability(&[100.0, 70.0], 0, false, 60, vec![]);
        assert!(!report.passed);
        assert!(report.performance_drop_pct > 25.0);
    }

    #[test]
    fn stability_passes_stable_runs() {
        let report = evaluate_stability(&[100.0, 98.0, 97.0], 0, false, 60, vec![]);
        assert!(report.passed);
    }
}
