mod cli;

use anyhow::Result;
use burd_bench::{
    DiskBenchmarkOptions, LlmBenchmarkOptions, NetworkBenchmarkOptions, ReportRunOptions,
    build_provider_details, build_raw_data, calculate_pricing, calculate_score, detect_health,
    estimate_earnings, generate_full_report, generate_signed_report, heartbeat_once, load_actions,
    load_logs, load_logs_for_task, load_signed_report_file, profile_for_vram, record_action,
    run_disk_benchmark, run_llm_benchmark, run_network_benchmark, run_stability_benchmark,
    save_latest_report, verify_provider, verify_signed_report,
};
use burd_hardware::{detect_specs, detect_system_report};
use burd_llmfit::build_fit_report;
use burd_protocol::{
    Challenge, ChallengeResponse, challenge_response_message, init_identity, load_identity,
    load_private_key, mock_challenge, rotate_identity_key, show_identity, sign_message,
    verify_challenge_response,
};
use clap::Parser;
use cli::{BenchCommands, ChallengeCommands, Cli, Commands, IdentityCommands};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::System { json: _ } => {
            let report = detect_system_report(AGENT_VERSION);
            let _ = record_action(
                "system detection",
                "completed",
                "Detect system",
                "Detected local hardware and runtime signals.",
                vec!["system report generated".to_string()],
            );
            print_json(&report)?;
        }
        Commands::Fit { json: _, limit } => {
            let specs = detect_specs();
            let report = build_fit_report(&specs, limit);
            let _ = record_action(
                "fit analysis",
                "completed",
                "Analyze model fit",
                "Analyzed model fit through llmfit adapter.",
                vec![format!("models analyzed: {}", report.total_models_analyzed)],
            );
            print_json(&report)?;
        }
        Commands::Bench { command } => match command {
            BenchCommands::Llm {
                provider,
                url,
                model,
                runs,
                profile,
                json: _,
            } => {
                let system = detect_system_report(AGENT_VERSION);
                let vram = system
                    .vram_total_gb
                    .or(system.vram_per_gpu_gb)
                    .unwrap_or(0.0);
                let report = run_llm_benchmark(LlmBenchmarkOptions {
                    provider,
                    url,
                    model,
                    runs,
                    profile,
                    detected_vram_gb: vram,
                });
                let _ = record_action(
                    "llm benchmark",
                    if report.passed { "completed" } else { "failed" },
                    "Run LLM benchmark",
                    "Ran local LLM benchmark through configured provider.",
                    report.errors.clone(),
                );
                print_json(&report)?;
            }
            BenchCommands::Stability {
                minutes,
                provider,
                url,
                model,
                json: _,
            } => {
                let system = detect_system_report(AGENT_VERSION);
                let vram = system
                    .vram_total_gb
                    .or(system.vram_per_gpu_gb)
                    .unwrap_or(0.0);
                let profile = profile_for_vram(vram);
                let report = run_stability_benchmark(
                    minutes,
                    LlmBenchmarkOptions {
                        provider,
                        url,
                        model,
                        runs: 1,
                        profile: Some(profile.id),
                        detected_vram_gb: vram,
                    },
                );
                let _ = record_action(
                    "stability benchmark",
                    if report.passed { "completed" } else { "failed" },
                    "Run stability benchmark",
                    "Looped local LLM benchmark to assess stability.",
                    report.runtime_errors.clone(),
                );
                print_json(&report)?;
            }
            BenchCommands::Network { endpoint, json: _ } => {
                let report = run_network_benchmark(NetworkBenchmarkOptions {
                    endpoint,
                    attempts: 5,
                });
                let _ = record_action(
                    "network benchmark",
                    if report.passed { "completed" } else { "failed" },
                    "Run network benchmark",
                    "Measured endpoint latency and failures.",
                    report.errors.clone(),
                );
                print_json(&report)?;
            }
            BenchCommands::Disk {
                directory,
                file_size_mb,
                json: _,
            } => {
                let report = run_disk_benchmark(DiskBenchmarkOptions {
                    directory: directory.unwrap_or_else(std::env::temp_dir),
                    file_size_mb,
                });
                let _ = record_action(
                    "disk benchmark",
                    if report.passed { "completed" } else { "failed" },
                    "Run disk benchmark",
                    "Measured local sequential disk read/write.",
                    report.errors.clone(),
                );
                print_json(&report)?;
            }
        },
        Commands::Score { json: _ } => {
            let specs = detect_specs();
            let system = detect_system_report(AGENT_VERSION);
            let fit = build_fit_report(&specs, Some(25));
            let report = calculate_score(&system, Some(&fit), None, None, None, None);
            let _ = record_action(
                "score calculation",
                "completed",
                "Calculate score",
                "Calculated Burd Compute Score.",
                vec![format!("score: {}", report.burd_compute_score)],
            );
            print_json(&report)?;
        }
        Commands::Report {
            json: _,
            run_all,
            signed,
            provider,
            url,
            model,
        } => {
            let mut options = ReportRunOptions::new(AGENT_VERSION);
            options.run_all = run_all;
            options.llm_provider = provider;
            options.llm_url = url;
            options.llm_model = model;
            if signed {
                let signed = generate_signed_report(options).map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "signed report generation",
                    if signed.signature_valid_locally {
                        "completed"
                    } else {
                        "failed"
                    },
                    "Generate signed report",
                    "Generated canonical report hash and Ed25519 signature.",
                    vec![format!("report_hash: {}", signed.report_hash)],
                );
                print_json(&signed)?;
            } else {
                let report = generate_full_report(options);
                let _ = save_latest_report(&report);
                let _ = record_action(
                    "report generation",
                    "completed",
                    "Generate report",
                    "Generated local provider validation report.",
                    vec!["report generated".to_string()],
                );
                print_json(&report)?;
            }
        }
        Commands::VerifyReport { file, json: _ } => {
            let report = load_signed_report_file(&file).map_err(anyhow::Error::msg)?;
            let result = verify_signed_report(&report);
            let _ = record_action(
                "report verification",
                if result.signature_valid {
                    "completed"
                } else {
                    "failed"
                },
                "Verify signed report",
                "Verified signed report hash and signature.",
                result.errors.clone(),
            );
            print_json(&result)?;
        }
        Commands::Identity { command } => match command {
            IdentityCommands::Init => {
                let result = init_identity().map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "identity init",
                    "completed",
                    "Initialize identity",
                    "Created or loaded provider and machine identity.",
                    vec![format!("provider_id: {}", result.identity.provider_id)],
                );
                print_json(&result)?;
            }
            IdentityCommands::Show { json: _ } => {
                print_json(&show_identity().map_err(anyhow::Error::msg)?)?;
            }
            IdentityCommands::RotateKey { confirm } => {
                let result = rotate_identity_key(confirm).map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "identity key rotation",
                    "completed",
                    "Rotate signing key",
                    "Rotated local Ed25519 signing key.",
                    vec![format!("provider_id: {}", result.provider_id)],
                );
                print_json(&result)?;
            }
        },
        Commands::Challenge { command } => match command {
            ChallengeCommands::CreateMock { profile, json: _ } => {
                let challenge = mock_challenge(&profile);
                print_json(&challenge)?;
            }
            ChallengeCommands::Run { file, json: _ } => {
                let challenge = read_json_file::<Challenge>(&file)?;
                let mut options = ReportRunOptions::new(AGENT_VERSION);
                options.run_all = true;
                options.challenge = Some(challenge.clone());
                let signed_report = generate_signed_report(options).map_err(anyhow::Error::msg)?;
                let config = load_identity().map_err(anyhow::Error::msg)?;
                let private_key = load_private_key(&config).map_err(anyhow::Error::msg)?;
                let message = challenge_response_message(
                    &challenge.challenge_id,
                    &challenge.nonce,
                    &config.provider_id,
                    &config.machine_id,
                    &signed_report.report_hash,
                )
                .map_err(anyhow::Error::msg)?;
                let signature = sign_message(&private_key.secret_key_base64, message.as_bytes())
                    .map_err(anyhow::Error::msg)?;
                let response = ChallengeResponse {
                    challenge_id: challenge.challenge_id.clone(),
                    nonce: challenge.nonce.clone(),
                    provider_id: config.provider_id,
                    machine_id: config.machine_id,
                    report_hash: signed_report.report_hash.clone(),
                    signature,
                    public_key: signed_report.public_key.clone(),
                    completed_at: chrono::Utc::now().to_rfc3339(),
                    status: "completed".to_string(),
                };
                let verification = verify_challenge_response(&challenge, &response);
                let output = ChallengeRunOutput {
                    challenge,
                    signed_report,
                    response,
                    verification,
                };
                let _ = record_action(
                    "challenge response",
                    if output.verification.valid {
                        "completed"
                    } else {
                        "failed"
                    },
                    "Run challenge",
                    "Ran local challenge and signed response.",
                    output.verification.errors.clone(),
                );
                print_json(&output)?;
            }
            ChallengeCommands::Verify { file, json: _ } => {
                let output = read_json_file::<ChallengeRunOutput>(&file)?;
                let verification = verify_challenge_response(&output.challenge, &output.response);
                print_json(&verification)?;
            }
        },
        Commands::Health { json: _ } => {
            print_json(&detect_health(AGENT_VERSION))?;
        }
        Commands::Heartbeat {
            once,
            interval,
            json: _,
        } => {
            if once || interval.is_none() {
                print_json(&heartbeat_once(AGENT_VERSION).map_err(anyhow::Error::msg)?)?;
            } else if let Some(seconds) = interval {
                run_heartbeat_loop(seconds)?;
            }
        }
        Commands::Provider { json: _, host_uri } => {
            print_json(&build_provider_details(AGENT_VERSION, &host_uri))?;
        }
        Commands::VerifyProvider { json: _ } => {
            print_json(&verify_provider(AGENT_VERSION))?;
        }
        Commands::Pricing { json: _ } => {
            let (system, score) = system_and_score();
            print_json(&calculate_pricing(&system, &score))?;
        }
        Commands::Earnings { json: _ } => {
            let (system, score) = system_and_score();
            let pricing = calculate_pricing(&system, &score);
            print_json(&estimate_earnings(&pricing))?;
        }
        Commands::Actions { json: _ } => {
            print_json(&load_actions().map_err(anyhow::Error::msg)?)?;
        }
        Commands::Logs { task, json: _ } => {
            if let Some(task_id) = task {
                print_json(&load_logs_for_task(&task_id).map_err(anyhow::Error::msg)?)?;
            } else {
                print_json(&load_logs().map_err(anyhow::Error::msg)?)?;
            }
        }
        Commands::Raw { json: _ } => {
            print_json(&build_raw_data(AGENT_VERSION, "http://127.0.0.1:8787"))?;
        }
        Commands::Serve { host, port } => {
            if host == "0.0.0.0" {
                eprintln!(
                    "Warning: serving on 0.0.0.0 exposes the local API beyond loopback; API token support is future work."
                );
            }
            burd_api_local::run_server(&host, port, AGENT_VERSION).map_err(anyhow::Error::msg)?;
        }
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn system_and_score() -> (burd_hardware::SystemReport, burd_bench::ScoreReport) {
    let specs = detect_specs();
    let system = detect_system_report(AGENT_VERSION);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    (system, score)
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn run_heartbeat_loop(seconds: u64) -> Result<()> {
    let interval = seconds.max(1);
    loop {
        let report = heartbeat_once(AGENT_VERSION).map_err(anyhow::Error::msg)?;
        println!("{}", serde_json::to_string(&report)?);
        std::thread::sleep(Duration::from_secs(interval));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChallengeRunOutput {
    challenge: Challenge,
    signed_report: burd_protocol::SignedReport,
    response: ChallengeResponse,
    verification: burd_protocol::ChallengeVerification,
}
