mod cli;

use anyhow::Result;
use burd_bench::{
    DiskBenchmarkOptions, LlmBenchmarkOptions, NetworkBenchmarkOptions, ReportRunOptions,
    calculate_score, generate_full_report, profile_for_vram, run_disk_benchmark, run_llm_benchmark,
    run_network_benchmark, run_stability_benchmark,
};
use burd_hardware::{detect_specs, detect_system_report};
use burd_llmfit::build_fit_report;
use burd_protocol::init_identity;
use clap::Parser;
use cli::{BenchCommands, Cli, Commands, IdentityCommands};

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
            print_json(&detect_system_report(AGENT_VERSION))?;
        }
        Commands::Fit { json: _, limit } => {
            let specs = detect_specs();
            print_json(&build_fit_report(&specs, limit))?;
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
                print_json(&report)?;
            }
            BenchCommands::Network { endpoint, json: _ } => {
                let report = run_network_benchmark(NetworkBenchmarkOptions {
                    endpoint,
                    attempts: 5,
                });
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
                print_json(&report)?;
            }
        },
        Commands::Score { json: _ } => {
            let specs = detect_specs();
            let system = detect_system_report(AGENT_VERSION);
            let fit = build_fit_report(&specs, Some(25));
            let report = calculate_score(&system, Some(&fit), None, None, None, None);
            print_json(&report)?;
        }
        Commands::Report {
            json: _,
            run_all,
            provider,
            url,
            model,
        } => {
            let mut options = ReportRunOptions::new(AGENT_VERSION);
            options.run_all = run_all;
            options.llm_provider = provider;
            options.llm_url = url;
            options.llm_model = model;
            print_json(&generate_full_report(options))?;
        }
        Commands::Identity { command } => match command {
            IdentityCommands::Init => {
                print_json(&init_identity().map_err(anyhow::Error::msg)?)?;
            }
        },
        Commands::Serve { host, port } => {
            burd_api_local::run_server(&host, port, AGENT_VERSION).map_err(anyhow::Error::msg)?;
        }
    }
    Ok(())
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
