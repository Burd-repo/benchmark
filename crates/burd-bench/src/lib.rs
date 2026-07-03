pub mod actions;
pub mod capability;
pub mod disk;
pub mod earnings;
pub mod health;
pub mod history;
pub mod llm;
pub mod network;
pub mod pricing;
pub mod profiles;
pub mod provider;
pub mod raw;
pub mod readiness;
pub mod registration;
pub mod report;
pub mod score;
pub mod session;
pub mod stability;
pub mod trust;
pub mod verification;
pub mod workload;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
pub(crate) mod test_fixtures;

pub use actions::{
    ActionStatus, Task, TaskLogs, load_actions, load_logs, load_logs_for_task, logs_summary,
    record_action,
};
pub use capability::{
    CapabilitySpotCheck, CapabilitySpotComponents, CapabilitySpotEvidence,
    CapabilitySpotVerificationReport, build_capability_spot_verification,
    build_capability_spot_verification_from, calculate_capability_spot_verification,
};
pub use burd_protocol::session::ProviderSessionStatusReport;
pub use disk::{DiskBenchmarkOptions, DiskBenchmarkReport, run_disk_benchmark};
pub use earnings::{EarningsReport, estimate_earnings};
pub use health::{
    HealthReport, HeartbeatReport, ReliabilityComponents, ReliabilityReport, UptimeCheck,
    UptimeHistory, UptimeSummary, calculate_reliability, clear_uptime_history, detect_health,
    heartbeat_once, load_reliability_report, load_uptime_summary,
};
pub use history::{
    BenchmarkHistoryClearResult, BenchmarkHistoryEntry, BenchmarkHistoryExportResult,
    BenchmarkHistoryLatest, BenchmarkHistoryList, append_report_history,
    append_signed_report_history, clear_history, export_history, history_summary,
    load_history_list, load_latest_history,
};
pub use llm::{LlmBenchmarkOptions, LlmBenchmarkReport, run_llm_benchmark};
pub use network::{
    NetworkBenchmarkOptions, NetworkBenchmarkReport, NetworkScoreComponents, NetworkScoreReport,
    calculate_network_score, load_latest_network_benchmark, load_network_score_report,
    network_score_from_report_value, run_network_benchmark, save_latest_network_benchmark,
};
pub use pricing::{PricingReport, calculate_pricing};
pub use profiles::{BenchmarkProfile, all_profiles, profile_for_vram};
pub use provider::{BurdProviderDetails, build_provider_details};
pub use raw::{RawData, build_raw_data};
pub use readiness::{
    ProviderReadiness, ProviderReadinessStatus, ReadinessCheck, ReadinessCheckStatus,
    ReadinessEvidenceStatus, ReadinessEvidenceSummary, build_provider_readiness,
};
pub use registration::{
    ProviderRegistrationExport, ProviderRegistrationPayload, build_registration_payload,
    export_registration_payload,
};
pub use report::{
    ReportRunOptions, generate_full_report, generate_signed_report, load_latest_signed_report,
    load_signed_report_file, save_latest_report, save_latest_signed_report, verify_signed_report,
};
pub use score::{ScoreReport, calculate_score, tier_for_score};
pub use session::{
    ProviderSessionExport, ProviderSessionStartOptions, build_provider_session_start,
    build_provider_session_status, export_provider_session_status, stop_provider_session,
};
pub use stability::{StabilityBenchmarkReport, run_stability_benchmark};
pub use trust::{TrustScoreReport, build_trust_score, calculate_trust_score};
pub use verification::{ProviderVerification, verify_provider};
pub use workload::{
    WorkloadEligibility, WorkloadEligibilityReport, build_workload_eligibility,
    calculate_workload_eligibility,
};







