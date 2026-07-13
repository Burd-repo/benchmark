#![recursion_limit = "256"]

pub mod benchmark_profile;
pub mod config;
pub mod db;
pub mod enrollment;
pub mod error;
pub mod evidence_registry;
pub mod http;
pub mod job_control;
pub mod marketplace;
pub mod metering;
pub mod migrations;
pub mod network_probe;
pub mod openapi;
pub mod proof_challenge;
pub mod rate_limit;
pub mod remote_session;
pub mod scheduler;
pub mod telemetry;
pub mod trust_policy;
pub mod verification_policy;
pub mod workload_policy;

pub use config::ControlPlaneConfig;
pub use db::{CreateProviderCommand, CreateProviderOutcome, Database, ProviderRecord};
pub use enrollment::EnrollmentError;
pub use error::{ApiError, ErrorCode};
pub use http::{AppState, router};
pub use remote_session::SessionError;
