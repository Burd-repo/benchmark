use serde::Serialize;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

pub const AGENT_EXIT_SCHEMA_VERSION: &str = "burd.agent.exit.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExitCategory {
    OperatorRequested,
    InvalidInvocation,
    LocalState,
    Unauthorized,
    Revoked,
    RemoteRejected,
    RemoteContract,
    Internal,
}

impl AgentExitCategory {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::OperatorRequested => 0,
            Self::InvalidInvocation => 2,
            Self::LocalState => 10,
            Self::Unauthorized => 11,
            Self::Revoked => 12,
            Self::RemoteRejected => 13,
            Self::RemoteContract => 14,
            Self::Internal => 15,
        }
    }

    const fn operator_message(self) -> &'static str {
        match self {
            Self::OperatorRequested => "Agent stopped after an operator request.",
            Self::InvalidInvocation => "Agent command arguments are invalid.",
            Self::LocalState => "Agent local state is invalid or unavailable.",
            Self::Unauthorized => {
                "Agent credentials are invalid, expired, or rejected by the Control Plane."
            }
            Self::Revoked => "Agent device or session was revoked by the Control Plane.",
            Self::RemoteRejected => "Control Plane rejected the Agent request.",
            Self::RemoteContract => "Control Plane response violated the Agent contract.",
            Self::Internal => "Agent terminated because of an internal runtime failure.",
        }
    }
}

pub struct AgentExitError {
    category: AgentExitCategory,
    failure_kind: &'static str,
    detail: String,
}

impl AgentExitError {
    pub fn from_failure_kind(failure_kind: &'static str, detail: impl Into<String>) -> Self {
        let category = match failure_kind {
            "local_state" | "lifecycle_state" | "state_lock" => AgentExitCategory::LocalState,
            "unauthorized" | "expired" => AgentExitCategory::Unauthorized,
            "revoked" | "session_revoked" => AgentExitCategory::Revoked,
            "control_plane_rejected" | "not_found" => AgentExitCategory::RemoteRejected,
            "control_plane_contract" => AgentExitCategory::RemoteContract,
            _ => AgentExitCategory::Internal,
        };
        Self {
            category,
            failure_kind,
            detail: detail.into(),
        }
    }

    pub fn invalid_invocation(failure_kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            category: AgentExitCategory::InvalidInvocation,
            failure_kind,
            detail: detail.into(),
        }
    }

    pub fn local_state(failure_kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            category: AgentExitCategory::LocalState,
            failure_kind,
            detail: detail.into(),
        }
    }

    pub fn internal(failure_kind: &'static str, detail: impl Into<String>) -> Self {
        Self {
            category: AgentExitCategory::Internal,
            failure_kind,
            detail: detail.into(),
        }
    }

    pub const fn category(&self) -> AgentExitCategory {
        self.category
    }

    pub const fn failure_kind(&self) -> &'static str {
        self.failure_kind
    }

    pub fn diagnostic_detail(&self) -> &str {
        &self.detail
    }

    pub const fn exit_code(&self) -> u8 {
        self.category.exit_code()
    }

    pub const fn event(&self) -> AgentExitEvent {
        AgentExitEvent {
            schema_version: AGENT_EXIT_SCHEMA_VERSION,
            event: "agent_exit",
            category: self.category,
            exit_code: self.exit_code(),
            failure_kind: Some(self.failure_kind),
            message: self.category.operator_message(),
        }
    }
}

impl fmt::Debug for AgentExitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentExitError")
            .field("category", &self.category)
            .field("failure_kind", &self.failure_kind)
            .finish_non_exhaustive()
    }
}

impl Display for AgentExitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.category.operator_message())
    }
}

impl Error for AgentExitError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AgentExitEvent {
    pub schema_version: &'static str,
    pub event: &'static str,
    pub category: AgentExitCategory,
    pub exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<&'static str>,
    pub message: &'static str,
}

impl AgentExitEvent {
    pub const fn operator_requested() -> Self {
        let category = AgentExitCategory::OperatorRequested;
        Self {
            schema_version: AGENT_EXIT_SCHEMA_VERSION,
            event: "agent_exit",
            category,
            exit_code: category.exit_code(),
            failure_kind: None,
            message: category.operator_message(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(AgentExitCategory::OperatorRequested.exit_code(), 0);
        assert_eq!(AgentExitCategory::InvalidInvocation.exit_code(), 2);
        assert_eq!(AgentExitCategory::LocalState.exit_code(), 10);
        assert_eq!(AgentExitCategory::Unauthorized.exit_code(), 11);
        assert_eq!(AgentExitCategory::Revoked.exit_code(), 12);
        assert_eq!(AgentExitCategory::RemoteRejected.exit_code(), 13);
        assert_eq!(AgentExitCategory::RemoteContract.exit_code(), 14);
        assert_eq!(AgentExitCategory::Internal.exit_code(), 15);
    }

    #[test]
    fn failure_kinds_map_to_operator_categories() {
        for (kind, expected) in [
            ("local_state", AgentExitCategory::LocalState),
            ("unauthorized", AgentExitCategory::Unauthorized),
            ("expired", AgentExitCategory::Unauthorized),
            ("revoked", AgentExitCategory::Revoked),
            ("session_revoked", AgentExitCategory::Revoked),
            ("not_found", AgentExitCategory::RemoteRejected),
            ("control_plane_rejected", AgentExitCategory::RemoteRejected),
            ("control_plane_contract", AgentExitCategory::RemoteContract),
            ("session_runtime", AgentExitCategory::Internal),
        ] {
            let error = AgentExitError::from_failure_kind(kind, "private diagnostic");
            assert_eq!(error.category(), expected, "{kind}");
        }
    }

    #[test]
    fn exit_event_omits_private_diagnostic_detail() {
        let error = AgentExitError::from_failure_kind(
            "unauthorized",
            "Authorization: Bearer device-secret",
        );
        let json = serde_json::to_string(&error.event()).unwrap();
        assert!(json.contains("\"schema_version\":\"burd.agent.exit.v1\""));
        assert!(json.contains("\"category\":\"unauthorized\""));
        assert!(json.contains("\"exit_code\":11"));
        assert!(json.contains("\"failure_kind\":\"unauthorized\""));
        assert!(!json.contains("device-secret"));
        assert!(!json.contains("Authorization"));
        assert!(!json.contains("Bearer"));
        let debug = format!("{error:?}");
        assert!(!debug.contains("device-secret"));
        assert!(!debug.contains("Authorization"));
        assert!(!debug.contains("Bearer"));
        let display = error.to_string();
        assert!(!display.contains("device-secret"));
        assert!(!display.contains("Authorization"));
        assert!(!display.contains("Bearer"));
        assert!(error.diagnostic_detail().contains("device-secret"));
    }
}
