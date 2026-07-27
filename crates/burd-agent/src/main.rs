mod cli;

use anyhow::Result;
use burd_agent::{
    AgentStateLock, AgentStateLockOperation, lifecycle, remote_enrollment, remote_session,
};
use burd_bench::{
    DiskBenchmarkOptions, LlmBenchmarkOptions, NetworkBenchmarkOptions, ReportRunOptions,
    SecureRuntimePlanOptions, append_report_history, append_signed_report_history,
    build_ai_performance_report, build_capability_spot_verification, build_provider_details,
    build_provider_readiness, build_provider_session_start, build_provider_session_status,
    build_raw_data, build_registration_payload, build_secure_runtime_plan, build_trust_score,
    build_workload_eligibility, calculate_network_score, calculate_pricing, calculate_score,
    clear_history, clear_uptime_history, detect_health, estimate_earnings, export_history,
    export_registration_payload, generate_full_report, generate_signed_report, heartbeat_once,
    load_actions, load_history_list, load_latest_history, load_logs, load_logs_for_task,
    load_network_score_report, load_reliability_report, load_signed_report_file,
    load_uptime_summary, profile_for_vram, record_action, run_disk_benchmark, run_llm_benchmark,
    run_network_benchmark, run_stability_benchmark, save_latest_network_benchmark,
    save_latest_report, stop_provider_session, verify_provider, verify_signed_report,
};
use burd_hardware::{
    build_hardware_fingerprint_report, build_system_report, detect_specs, detect_system_report,
};
use burd_llmfit::build_fit_report;
use burd_protocol::{
    Challenge, ChallengeResponse, ChallengeRunOutput, challenge_response_message_with_fingerprint,
    create_api_token, evidence_freshness_from_window, init_identity, load_identity,
    load_private_key, migrate_identity, mock_challenge, rotate_api_token, rotate_identity_key,
    save_latest_challenge_output, show_api_token_status, show_identity, show_remote_enrollment,
    sign_message, verify_challenge_response,
};
use clap::Parser;
use cli::{
    ApiTokenCommands, BenchCommands, ChallengeCommands, Cli, Commands, EnrollmentCommands,
    HistoryCommands, IdentityCommands, RemoteSessionCommands, RuntimeCommands, UptimeCommands,
};
use serde::Deserialize;
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
    let _maintenance_lock = maintenance_lock_operation(&cli.command)
        .map(AgentStateLock::acquire)
        .transpose()
        .map_err(anyhow::Error::msg)?;
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
        Commands::Fingerprint { json: _ } => {
            let system = detect_system_report(AGENT_VERSION);
            let report = build_hardware_fingerprint_report(&system);
            let _ = record_action(
                "hardware fingerprint",
                "completed",
                "Build hardware fingerprint",
                "Built stable hardware fingerprint and marketplace GPU policy snapshot.",
                vec![format!(
                    "marketplace_eligible: {}",
                    report.marketplace_policy.marketplace_eligible
                )],
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
                let endpoint = endpoint
                    .or_else(|| {
                        load_identity()
                            .ok()
                            .map(|config| config.default_network_endpoint)
                    })
                    .unwrap_or_else(|| "https://www.cloudflare.com/cdn-cgi/trace".to_string());
                let report = run_network_benchmark(NetworkBenchmarkOptions {
                    endpoint,
                    attempts: 5,
                });
                let _ = save_latest_network_benchmark(&report);
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
            let system = build_system_report(&specs, AGENT_VERSION);
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
                let _ = save_latest_report(&signed.report);
                if run_all {
                    let _ = append_signed_report_history(&signed);
                }
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
                if run_all {
                    let _ = append_report_history(&report);
                }
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
            IdentityCommands::Migrate { from, confirm } => {
                let result =
                    migrate_identity(from.as_deref(), confirm).map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "identity migration",
                    "completed",
                    "Migrate identity",
                    "Normalized, repaired, or imported local provider state after backup.",
                    result.warnings.clone(),
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
        Commands::Enrollment { command } => match command {
            EnrollmentCommands::Enroll { control_plane_url } => {
                let token = std::env::var("BURD_ENROLLMENT_TOKEN").map_err(|_| {
                    anyhow::anyhow!(
                        "BURD_ENROLLMENT_TOKEN is required and is consumed as a one-time secret"
                    )
                })?;
                let result = remote_enrollment::enroll(&control_plane_url, token, AGENT_VERSION)
                    .map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "remote enrollment",
                    "completed",
                    "Enroll provider device",
                    "Completed backend Ed25519 possession proof.",
                    vec![
                        format!("provider_id: {}", result.provider_id),
                        format!("device_id: {}", result.device_id),
                    ],
                );
                print_json(&result)?;
            }
            EnrollmentCommands::Status { json: _ } => {
                print_json(&show_remote_enrollment().map_err(anyhow::Error::msg)?)?;
            }
            EnrollmentCommands::RefreshCredential { json: _ } => {
                let result = remote_enrollment::refresh_credential().map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "device credential refresh",
                    "completed",
                    "Refresh device credential",
                    "Rotated the short-lived backend device credential.",
                    vec![format!("device_id: {}", result.device_id)],
                );
                print_json(&result)?;
            }
        },
        Commands::RemoteSession { command } => match command {
            RemoteSessionCommands::Connect {
                max_reconnect_delay_seconds,
                telemetry,
                telemetry_batch_samples,
                proofs,
            } => {
                let result = remote_session::connect(
                    AGENT_VERSION,
                    max_reconnect_delay_seconds,
                    telemetry,
                    telemetry_batch_samples,
                    proofs,
                )
                .map_err(anyhow::Error::msg)?;
                print_json(&result)?;
            }
            RemoteSessionCommands::Status { json: _ } => {
                print_json(&remote_session::status().map_err(anyhow::Error::msg)?)?;
            }
            RemoteSessionCommands::Lifecycle { json: _ } => {
                print_json(&lifecycle::lifecycle_status().map_err(anyhow::Error::msg)?)?;
            }
        },
        Commands::Challenge { command } => match command {
            ChallengeCommands::CreateMock { profile, json: _ } => {
                let challenge = mock_challenge(&profile);
                print_json(&challenge)?;
            }
            ChallengeCommands::RunLocal { profile, json: _ } => {
                let output = run_challenge(mock_challenge(&profile))?;
                print_json(&output)?;
            }
            ChallengeCommands::Run { file, json: _ } => {
                let challenge = read_json_file::<Challenge>(&file)?;
                let output = run_challenge(challenge)?;
                print_json(&output)?;
            }
            ChallengeCommands::Verify { file, json: _ } => {
                let output = read_json_file::<ChallengeRunOutput>(&file)?;
                let verification = verify_challenge_response(&output.challenge, &output.response);
                print_json(&verification)?;
            }
        },
        Commands::Session { command } => match command {
            cli::SessionCommands::Start { json: _ } => {
                let report = build_provider_session_start(AGENT_VERSION, "http://127.0.0.1:8787")
                    .map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "provider session",
                    "completed",
                    "Start provider session",
                    "Created a local provider session snapshot.",
                    report.warnings.clone(),
                );
                print_json(&report)?;
            }
            cli::SessionCommands::Status { json: _ } => {
                let report = build_provider_session_status(AGENT_VERSION, "http://127.0.0.1:8787")
                    .map_err(anyhow::Error::msg)?;
                print_json(&report)?;
            }
            cli::SessionCommands::Stop { json: _ } => {
                let report = stop_provider_session().map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "provider session",
                    "completed",
                    "Stop provider session",
                    "Stopped the local provider session snapshot.",
                    report.warnings.clone(),
                );
                print_json(&report)?;
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
        Commands::Uptime { json: _, command } => match command {
            Some(UptimeCommands::Clear { confirm }) => {
                let result = clear_uptime_history(confirm).map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "uptime clear",
                    "completed",
                    "Clear uptime history",
                    "Cleared local uptime history.",
                    vec!["uptime history cleared".to_string()],
                );
                print_json(&result)?;
            }
            None => {
                print_json(&load_uptime_summary().map_err(anyhow::Error::msg)?)?;
            }
        },
        Commands::Reliability { json: _ } => {
            print_json(&load_reliability_report().map_err(anyhow::Error::msg)?)?;
        }
        Commands::NetworkScore { json: _ } => {
            let report =
                load_network_score_report().unwrap_or_else(|_| calculate_network_score(None));
            print_json(&report)?;
        }
        Commands::TrustScore { json: _ } => {
            print_json(&build_trust_score(AGENT_VERSION))?;
        }
        Commands::AiPerformance { json: _ } => {
            print_json(&build_ai_performance_report(AGENT_VERSION))?;
        }
        Commands::CapabilitySpot { json: _ } => {
            print_json(&build_capability_spot_verification(AGENT_VERSION))?;
        }
        Commands::WorkloadEligibility { json: _ } => {
            print_json(&build_workload_eligibility(AGENT_VERSION))?;
        }
        Commands::Runtime { command } => match command {
            RuntimeCommands::Check { json: _ } => {
                let plan =
                    build_secure_runtime_plan(AGENT_VERSION, SecureRuntimePlanOptions::default());
                let _ = record_action(
                    "secure runtime check",
                    runtime_action_status(&plan),
                    "Check secure runtime",
                    "Inspected local Docker/NVIDIA secure runtime readiness.",
                    runtime_action_details(&plan),
                );
                print_json(&plan)?;
            }
            RuntimeCommands::Plan {
                image_ref,
                gpu_uuid,
                allow_image_ref,
                template_id,
                cpu_count,
                memory_mib,
                json: _,
            } => {
                let defaults = SecureRuntimePlanOptions::default();
                let options = SecureRuntimePlanOptions {
                    template_id,
                    image_ref: Some(image_ref),
                    gpu_uuid,
                    allowed_image_refs: allow_image_ref,
                    cpu_count: cpu_count.or(defaults.cpu_count),
                    memory_mib: memory_mib.or(defaults.memory_mib),
                    pids_limit: defaults.pids_limit,
                    shm_size_mib: defaults.shm_size_mib,
                };
                let plan = build_secure_runtime_plan(AGENT_VERSION, options);
                let _ = record_action(
                    "secure runtime plan",
                    runtime_action_status(&plan),
                    "Plan secure runtime",
                    "Built Docker/NVIDIA sandbox plan for an allowlisted runtime image.",
                    runtime_action_details(&plan),
                );
                print_json(&plan)?;
            }
        },
        Commands::Provider { json: _, host_uri } => {
            print_json(&build_provider_details(AGENT_VERSION, &host_uri))?;
        }
        Commands::VerifyProvider { json: _ } => {
            print_json(&verify_provider(AGENT_VERSION))?;
        }
        Commands::Readiness { json } => {
            let readiness = build_provider_readiness(AGENT_VERSION, "http://127.0.0.1:8787");
            if json {
                print_json(&readiness)?;
            } else {
                print_readiness(&readiness);
            }
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
        Commands::Logs {
            task,
            tail,
            json: _,
        } => {
            if let Some(task_id) = task {
                let mut logs = load_logs_for_task(&task_id).map_err(anyhow::Error::msg)?;
                if let Some(limit) = tail {
                    logs = tail_items(logs, limit);
                }
                print_json(&logs)?;
            } else {
                let mut logs = load_logs().map_err(anyhow::Error::msg)?;
                if let Some(limit) = tail {
                    logs = tail_items(logs, limit);
                }
                print_json(&logs)?;
            }
        }
        Commands::History { json: _, command } => match command {
            Some(HistoryCommands::Latest { json: _ }) => {
                print_json(&load_latest_history().map_err(anyhow::Error::msg)?)?;
            }
            Some(HistoryCommands::Clear { confirm }) => {
                let result = clear_history(confirm).map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "benchmark history clear",
                    "completed",
                    "Clear benchmark history",
                    "Cleared local benchmark history.",
                    vec![format!("entries_removed: {}", result.entries_removed)],
                );
                print_json(&result)?;
            }
            Some(HistoryCommands::Export { output }) => {
                let result = export_history(&output).map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "benchmark history export",
                    "completed",
                    "Export benchmark history",
                    "Exported local benchmark history to a JSON file.",
                    vec![format!("output: {}", result.output)],
                );
                print_json(&result)?;
            }
            None => {
                print_json(&load_history_list().map_err(anyhow::Error::msg)?)?;
            }
        },
        Commands::ApiToken { command } => match command {
            ApiTokenCommands::Create { json: _ } => {
                let result = create_api_token().map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "api token create",
                    "completed",
                    "Create API token",
                    "Created local API bearer token hash.",
                    vec!["token shown once".to_string()],
                );
                print_json(&result)?;
            }
            ApiTokenCommands::Rotate { json: _ } => {
                let result = rotate_api_token().map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "api token rotate",
                    "completed",
                    "Rotate API token",
                    "Rotated local API bearer token hash.",
                    vec!["token shown once".to_string()],
                );
                print_json(&result)?;
            }
            ApiTokenCommands::Show { json: _ } => {
                print_json(&show_api_token_status().map_err(anyhow::Error::msg)?)?;
            }
        },
        Commands::RegistrationPayload { json: _, output } => {
            if let Some(path) = output {
                let result = export_registration_payload(AGENT_VERSION, &path)
                    .map_err(anyhow::Error::msg)?;
                let _ = record_action(
                    "registration payload",
                    "completed",
                    "Export registration payload",
                    "Generated provider registration payload for future Burd backend.",
                    vec![format!("output: {}", result.output)],
                );
                print_json(&result.payload)?;
            } else {
                let payload = build_registration_payload(AGENT_VERSION);
                let _ = record_action(
                    "registration payload",
                    "completed",
                    "Build registration payload",
                    "Generated provider registration payload for future Burd backend.",
                    vec!["output: stdout".to_string()],
                );
                print_json(&payload)?;
            }
        }
        Commands::Raw { json: _ } => {
            print_json(&build_raw_data(AGENT_VERSION, "http://127.0.0.1:8787"))?;
        }
        Commands::Serve { host, port } => {
            if host == "0.0.0.0" {
                eprintln!(
                    "Warning: serving on 0.0.0.0 exposes the local API beyond loopback; create an API token with `burd-agent api-token create --json` and send Authorization: Bearer <token>."
                );
            }
            burd_api_local::run_server(&host, port, AGENT_VERSION).map_err(anyhow::Error::msg)?;
        }
    }
    Ok(())
}

fn maintenance_lock_operation(command: &Commands) -> Option<AgentStateLockOperation> {
    match command {
        Commands::Identity {
            command: IdentityCommands::Init,
        } => Some(AgentStateLockOperation::IdentityInit),
        Commands::Identity {
            command: IdentityCommands::Migrate { .. },
        } => Some(AgentStateLockOperation::IdentityMigrate),
        Commands::Identity {
            command: IdentityCommands::RotateKey { .. },
        } => Some(AgentStateLockOperation::IdentityRotateKey),
        Commands::Enrollment {
            command: EnrollmentCommands::Enroll { .. },
        } => Some(AgentStateLockOperation::EnrollmentEnroll),
        Commands::Enrollment {
            command: EnrollmentCommands::RefreshCredential { .. },
        } => Some(AgentStateLockOperation::EnrollmentRefreshCredential),
        Commands::ApiToken {
            command: ApiTokenCommands::Create { .. },
        } => Some(AgentStateLockOperation::ApiTokenCreate),
        Commands::ApiToken {
            command: ApiTokenCommands::Rotate { .. },
        } => Some(AgentStateLockOperation::ApiTokenRotate),
        _ => None,
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn runtime_action_status(plan: &burd_protocol::SecureRuntimePlan) -> &'static str {
    match plan.status.as_str() {
        "ready" => "completed",
        "verification_required" | "unsupported_host" => "partial",
        _ => "failed",
    }
}

fn runtime_action_details(plan: &burd_protocol::SecureRuntimePlan) -> Vec<String> {
    let mut details = vec![format!("status: {}", plan.status)];
    details.extend(
        plan.checks
            .iter()
            .filter(|check| check.status != "passed")
            .map(|check| format!("{}: {}", check.id, check.status)),
    );
    details
}

fn print_readiness(readiness: &burd_bench::ProviderReadiness) {
    println!(
        "Provider readiness: {} ({}/100)",
        readiness.readiness_level, readiness.readiness_score
    );
    println!("Status: {}", readiness.status.as_str());
    println!("State directory: {}", readiness.state.state_dir);
    println!("Config path: {}", readiness.state.config_path);
    for warning in &readiness.state.warnings {
        println!("State warning: {warning}");
    }
    println!();
    println!("Checks:");
    for check in &readiness.checks {
        println!(
            "- [{}] {}: {} ({}/{})",
            check.status.as_str(),
            check.label,
            check.message,
            check.score,
            check.max_score
        );
    }
    if !readiness.warnings.is_empty() {
        println!();
        println!("Warnings:");
        for warning in &readiness.warnings {
            println!("- {warning}");
        }
    }
    if !readiness.recommendations.is_empty() {
        println!();
        println!("Recommendations:");
        for recommendation in &readiness.recommendations {
            println!("- {recommendation}");
        }
    }
}

fn system_and_score() -> (burd_hardware::SystemReport, burd_bench::ScoreReport) {
    let specs = detect_specs();
    let system = build_system_report(&specs, AGENT_VERSION);
    let fit = build_fit_report(&specs, Some(25));
    let score = calculate_score(&system, Some(&fit), None, None, None, None);
    (system, score)
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn tail_items<T>(mut items: Vec<T>, limit: usize) -> Vec<T> {
    if items.len() <= limit {
        return items;
    }
    items.drain(0..items.len() - limit);
    items
}

fn run_heartbeat_loop(seconds: u64) -> Result<()> {
    let interval = seconds.max(1);
    loop {
        let report = heartbeat_once(AGENT_VERSION).map_err(anyhow::Error::msg)?;
        println!("{}", serde_json::to_string(&report)?);
        std::thread::sleep(Duration::from_secs(interval));
    }
}

fn run_challenge(challenge: Challenge) -> Result<ChallengeRunOutput> {
    let mut options = ReportRunOptions::new(AGENT_VERSION);
    options.run_all = true;
    options.challenge = Some(challenge.clone());
    let signed_report = generate_signed_report(options).map_err(anyhow::Error::msg)?;
    let _ = save_latest_report(&signed_report.report);
    let _ = append_signed_report_history(&signed_report);
    let config = load_identity().map_err(anyhow::Error::msg)?;
    let private_key = load_private_key(&config).map_err(anyhow::Error::msg)?;
    let hardware_fingerprint = signed_report
        .report
        .hardware_fingerprint
        .clone()
        .ok_or_else(|| anyhow::anyhow!("signed report does not include hardware fingerprint"))?;
    let message = challenge_response_message_with_fingerprint(
        &challenge.challenge_id,
        &challenge.nonce,
        &config.provider_id,
        &config.machine_id,
        &signed_report.report_hash,
        &hardware_fingerprint,
    )
    .map_err(anyhow::Error::msg)?;
    let signature = sign_message(&private_key.secret_key_base64, message.as_bytes())
        .map_err(anyhow::Error::msg)?;
    let completed_at = chrono::Utc::now().to_rfc3339();
    let response_evidence =
        evidence_freshness_from_window(&challenge.issued_at, &challenge.expires_at)
            .map_err(anyhow::Error::msg)?;
    let mut response = ChallengeResponse {
        challenge_id: challenge.challenge_id.clone(),
        nonce: challenge.nonce.clone(),
        provider_id: config.provider_id,
        machine_id: config.machine_id,
        report_hash: signed_report.report_hash.clone(),
        hardware_fingerprint: Some(hardware_fingerprint),
        signed_report: Some(signed_report.clone()),
        signature,
        public_key: signed_report.public_key.clone(),
        completed_at,
        issued_at: response_evidence.issued_at,
        expires_at: response_evidence.expires_at,
        is_expired: response_evidence.is_expired,
        age_seconds: response_evidence.age_seconds,
        ttl_seconds: response_evidence.ttl_seconds,
        status: "partial".to_string(),
        failed_requirements: Vec::new(),
        verification_result: None,
    };
    let verification = verify_challenge_response(&challenge, &response);
    response.status = if verification.expired {
        "expired".to_string()
    } else if verification.valid {
        "passed".to_string()
    } else {
        "failed".to_string()
    };
    response.failed_requirements = verification.errors.clone();
    response.verification_result = Some(serde_json::json!({
        "valid": verification.valid,
        "signature_valid": verification.signature_valid,
        "expired": verification.expired,
        "evidence": verification.evidence.clone(),
        "checked_at": verification.checked_at.clone(),
        "warnings": verification.warnings.clone(),
        "errors": verification.errors.clone(),
    }));
    let output = ChallengeRunOutput {
        challenge,
        signed_report,
        response,
        verification,
    };
    save_latest_challenge_output(&output).map_err(anyhow::Error::msg)?;
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
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_state_mutations_require_the_maintenance_lock() {
        let cases: &[(&[&str], AgentStateLockOperation)] = &[
            (&["identity", "init"], AgentStateLockOperation::IdentityInit),
            (
                &["identity", "migrate"],
                AgentStateLockOperation::IdentityMigrate,
            ),
            (
                &["identity", "rotate-key"],
                AgentStateLockOperation::IdentityRotateKey,
            ),
            (
                &["enrollment", "enroll"],
                AgentStateLockOperation::EnrollmentEnroll,
            ),
            (
                &["enrollment", "refresh-credential"],
                AgentStateLockOperation::EnrollmentRefreshCredential,
            ),
            (
                &["api-token", "create"],
                AgentStateLockOperation::ApiTokenCreate,
            ),
            (
                &["api-token", "rotate"],
                AgentStateLockOperation::ApiTokenRotate,
            ),
        ];

        for (args, expected) in cases {
            assert_eq!(operation_for(args), Some(*expected), "{args:?}");
        }
    }

    #[test]
    fn read_only_and_self_locked_commands_remain_available() {
        for args in [
            &["identity", "show"][..],
            &["enrollment", "status"][..],
            &["remote-session", "status"][..],
            &["remote-session", "lifecycle"][..],
            &["remote-session", "connect"][..],
            &["api-token", "show"][..],
            &["system"][..],
        ] {
            assert_eq!(operation_for(args), None, "{args:?}");
        }
    }

    fn operation_for(args: &[&str]) -> Option<AgentStateLockOperation> {
        let cli =
            Cli::try_parse_from(std::iter::once("burd-agent").chain(args.iter().copied())).unwrap();
        maintenance_lock_operation(&cli.command)
    }
}
