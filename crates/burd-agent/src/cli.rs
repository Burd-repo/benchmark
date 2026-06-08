use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "burd-agent")]
#[command(about = "Burd local GPU benchmark and provider validation agent")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Detect local hardware and runtime signals.
    System {
        #[arg(long)]
        json: bool,
    },
    /// Analyze which models fit this hardware using the llmfit adapter.
    Fit {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Run benchmark groups.
    Bench {
        #[command(subcommand)]
        command: BenchCommands,
    },
    /// Calculate Burd Compute Score.
    Score {
        #[arg(long)]
        json: bool,
    },
    /// Generate a full provider validation report.
    Report {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        run_all: bool,
        #[arg(long)]
        signed: bool,
        #[arg(long, default_value = "ollama")]
        provider: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Verify a signed report file.
    VerifyReport {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Manage local agent identity.
    Identity {
        #[command(subcommand)]
        command: IdentityCommands,
    },
    /// Create, run or verify benchmark challenges.
    Challenge {
        #[command(subcommand)]
        command: ChallengeCommands,
    },
    /// Show local provider health.
    Health {
        #[arg(long)]
        json: bool,
    },
    /// Record provider heartbeat checks.
    Heartbeat {
        #[arg(long)]
        once: bool,
        #[arg(long)]
        interval: Option<u64>,
        #[arg(long)]
        json: bool,
    },
    /// Aggregate local provider console details.
    Provider {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "http://127.0.0.1:8787")]
        host_uri: String,
    },
    /// Show provider verification status.
    VerifyProvider {
        #[arg(long)]
        json: bool,
    },
    /// Show demonstrative pricing.
    Pricing {
        #[arg(long)]
        json: bool,
    },
    /// Show demonstrative earnings estimate.
    Earnings {
        #[arg(long)]
        json: bool,
    },
    /// Show local actions.
    Actions {
        #[arg(long)]
        json: bool,
    },
    /// Show local logs.
    Logs {
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show redacted raw provider console data.
    Raw {
        #[arg(long)]
        json: bool,
    },
    /// Serve local API and benchmark UI.
    Serve {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
}

#[derive(Debug, Subcommand)]
pub enum BenchCommands {
    /// Run real LLM benchmark through Ollama, vLLM, MLX or auto-detect.
    Llm {
        #[arg(long, default_value = "ollama")]
        provider: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run stability benchmark by looping LLM benchmark.
    Stability {
        #[arg(long, default_value_t = 10)]
        minutes: u64,
        #[arg(long, default_value = "ollama")]
        provider: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Measure endpoint latency and failures.
    Network {
        #[arg(long, default_value = "https://www.cloudflare.com/cdn-cgi/trace")]
        endpoint: String,
        #[arg(long)]
        json: bool,
    },
    /// Measure approximate sequential disk read/write.
    Disk {
        #[arg(long)]
        directory: Option<PathBuf>,
        #[arg(long, default_value_t = 32)]
        file_size_mb: u64,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum IdentityCommands {
    /// Create ~/.burd/agent.json and a local Ed25519 private key.
    Init,
    /// Show public identity status.
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Rotate the local signing key.
    RotateKey {
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ChallengeCommands {
    /// Create a mock backend challenge.
    CreateMock {
        #[arg(long, default_value = "profile_8gb")]
        profile: String,
        #[arg(long)]
        json: bool,
    },
    /// Run a local challenge file and sign the response.
    Run {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Verify a signed challenge response file.
    Verify {
        #[arg(long)]
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
}
