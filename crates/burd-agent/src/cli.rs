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
    /// Build the stable hardware fingerprint and marketplace GPU policy snapshot.
    Fingerprint {
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
    /// Enroll this agent with the Burd control plane.
    Enrollment {
        #[command(subcommand)]
        command: EnrollmentCommands,
    },
    /// Maintain the authenticated outbound control-plane session.
    RemoteSession {
        #[command(subcommand)]
        command: RemoteSessionCommands,
    },
    /// Create, run or verify benchmark challenges.
    Challenge {
        #[command(subcommand)]
        command: ChallengeCommands,
    },
    /// Manage local provider session state.
    Session {
        #[command(subcommand)]
        command: SessionCommands,
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
    /// Show or clear persisted uptime history.
    Uptime {
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<UptimeCommands>,
    },
    /// Calculate local reliability from heartbeat uptime history.
    Reliability {
        #[arg(long)]
        json: bool,
    },
    /// Calculate local network score from the latest finite network benchmark.
    NetworkScore {
        #[arg(long)]
        json: bool,
    },
    /// Calculate local heuristic trust score from verification, freshness, reliability, network, and history.
    TrustScore {
        #[arg(long)]
        json: bool,
    },
    /// Show consolidated AI performance metrics without running a benchmark.
    AiPerformance {
        #[arg(long)]
        json: bool,
    },
    /// Run local/mock AI capability spot verification from fit, runtime, signed evidence, and optional live benchmark proof.
    CapabilitySpot {
        #[arg(long)]
        json: bool,
    },
    /// Evaluate local and future marketplace workload eligibility.
    WorkloadEligibility {
        #[arg(long)]
        json: bool,
    },
    /// Inspect or plan the secure provider runtime sandbox.
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommands,
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
    /// Consolidate local provider readiness checks.
    Readiness {
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
        tail: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    /// Show, export or clear benchmark history.
    History {
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        command: Option<HistoryCommands>,
    },
    /// Manage local API bearer token.
    ApiToken {
        #[command(subcommand)]
        command: ApiTokenCommands,
    },
    /// Build provider registration payload for future Burd backend registration.
    RegistrationPayload {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        output: Option<PathBuf>,
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
        #[arg(long)]
        endpoint: Option<String>,
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
pub enum RuntimeCommands {
    /// Check whether this host can prepare a secure GPU runtime.
    Check {
        #[arg(long)]
        json: bool,
    },
    /// Build a Docker/NVIDIA sandbox plan for an approved runtime image.
    Plan {
        #[arg(long)]
        image_ref: String,
        #[arg(long)]
        gpu_uuid: Option<String>,
        #[arg(long)]
        allow_image_ref: Vec<String>,
        #[arg(long, default_value = "llm_inference")]
        template_id: String,
        #[arg(long)]
        cpu_count: Option<f64>,
        #[arg(long)]
        memory_mib: Option<u64>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum IdentityCommands {
    /// Create ~/.burd/agent.json and a local Ed25519 private key.
    Init,
    /// Safely normalize, repair, or import an identity state after creating a backup.
    Migrate {
        #[arg(long)]
        from: Option<PathBuf>,
        #[arg(long)]
        confirm: bool,
    },
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
pub enum EnrollmentCommands {
    /// Complete remote enrollment using BURD_ENROLLMENT_TOKEN.
    Enroll {
        #[arg(long, default_value = "https://api.burd.cloud")]
        control_plane_url: String,
    },
    /// Show remote identity and credential expiry without exposing the credential.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Rotate the short-lived device credential.
    RefreshCredential {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RemoteSessionCommands {
    /// Connect to the backend and maintain sequenced heartbeats until interrupted.
    Connect {
        #[arg(long, default_value_t = 30)]
        max_reconnect_delay_seconds: u64,
        #[arg(long)]
        telemetry: bool,
        #[arg(long, default_value_t = 8)]
        telemetry_batch_samples: usize,
        /// Execute backend-issued Proof of Capability challenges.
        #[arg(long)]
        proofs: bool,
    },
    /// Read the backend-authoritative state of the persisted remote session.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Read the local foreground process lifecycle and readiness state.
    Lifecycle {
        #[arg(long)]
        json: bool,
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
    /// Create and run a local mock challenge without intermediate files.
    RunLocal {
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

#[derive(Debug, Subcommand)]
pub enum SessionCommands {
    /// Start a local provider session snapshot.
    Start {
        #[arg(long)]
        json: bool,
    },
    /// Show the current local provider session snapshot.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Stop the current local provider session snapshot.
    Stop {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum HistoryCommands {
    /// Show the latest benchmark history entry.
    Latest {
        #[arg(long)]
        json: bool,
    },
    /// Clear benchmark history.
    Clear {
        #[arg(long)]
        confirm: bool,
    },
    /// Export benchmark history to a JSON file.
    Export {
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum ApiTokenCommands {
    /// Create an API token and print it once.
    Create {
        #[arg(long)]
        json: bool,
    },
    /// Rotate the API token and print it once.
    Rotate {
        #[arg(long)]
        json: bool,
    },
    /// Show API token status without printing the token.
    Show {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum UptimeCommands {
    /// Clear persisted uptime history.
    Clear {
        #[arg(long)]
        confirm: bool,
    },
}
