//! Safe external connector contracts.
//!
//! Connectors are deliberately inactive infrastructure: Conclave can validate
//! whether a future read-only tool call is allowed, but this module does not
//! perform network I/O or invoke any external system.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use conclave_core::{Error, Result};

use crate::privacy::{sha256_hex, DataBoundaryMode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorConfig {
    pub name: String,
    pub url: String,
    pub auth: ConnectorAuth,
    pub phi_safe: bool,
    pub tools: Vec<String>,
    pub timeout_ms: u64,
    pub logging_policy: ConnectorLoggingPolicy,
    /// Connectors are disabled by default and must be explicitly enabled by a
    /// future settings surface or workspace policy file.
    pub enabled: bool,
}

impl ConnectorConfig {
    pub fn disabled(name: impl Into<String>, url: impl Into<String>, tools: Vec<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            auth: ConnectorAuth::None,
            phi_safe: false,
            tools,
            timeout_ms: 10_000,
            logging_policy: ConnectorLoggingPolicy::FingerprintOnly,
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAuth {
    None,
    BearerEnv {
        env_var: String,
    },
    BasicEnv {
        username_env: String,
        password_env: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorLoggingPolicy {
    None,
    MetadataOnly,
    FingerprintOnly,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRegistry {
    connectors: BTreeMap<String, ConnectorConfig>,
}

impl ConnectorRegistry {
    pub fn new(configs: Vec<ConnectorConfig>) -> Self {
        let connectors = configs
            .into_iter()
            .map(|c| (c.name.clone(), c))
            .collect::<BTreeMap<_, _>>();
        Self { connectors }
    }

    pub fn is_empty(&self) -> bool {
        self.connectors.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&ConnectorConfig> {
        self.connectors.get(name)
    }

    pub fn enabled(&self) -> impl Iterator<Item = &ConnectorConfig> {
        self.connectors.values().filter(|c| c.enabled)
    }

    pub fn validate_call(&self, request: &ConnectorCallRequest<'_>) -> Result<ConnectorCallPlan> {
        let connector = self.get(request.connector_name).ok_or_else(|| {
            Error::invalid_config(format!("connector `{}` not found", request.connector_name))
        })?;
        if !connector.enabled {
            return Err(Error::invalid_config(format!(
                "connector `{}` is disabled",
                connector.name
            )));
        }
        if !connector.tools.iter().any(|tool| tool == request.tool_name) {
            return Err(Error::invalid_config(format!(
                "connector `{}` does not expose tool `{}`",
                connector.name, request.tool_name
            )));
        }
        if matches!(request.data_boundary_mode, DataBoundaryMode::LocalOnly) {
            return Err(Error::invalid_config(
                "local_only blocks external connector calls",
            ));
        }
        if request.carries_phi && !connector.phi_safe {
            return Err(Error::invalid_config(format!(
                "connector `{}` does not declare phi_safe=true",
                connector.name
            )));
        }
        if request.carries_phi && !request.user_confirmed_phi {
            return Err(Error::invalid_config(
                "PHI connector call requires explicit per-call confirmation",
            ));
        }
        Ok(ConnectorCallPlan {
            connector_name: connector.name.clone(),
            logical_endpoint: connector.url.clone(),
            tool_name: request.tool_name.to_owned(),
            timeout_ms: connector.timeout_ms,
            logging_policy: connector.logging_policy,
            carries_phi: request.carries_phi,
            input_sha256: sha256_hex(request.input_fingerprint_material),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConnectorCallRequest<'a> {
    pub connector_name: &'a str,
    pub tool_name: &'a str,
    pub data_boundary_mode: DataBoundaryMode,
    pub carries_phi: bool,
    pub user_confirmed_phi: bool,
    pub input_fingerprint_material: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorCallPlan {
    pub connector_name: String,
    pub logical_endpoint: String,
    pub tool_name: String,
    pub timeout_ms: u64,
    pub logging_policy: ConnectorLoggingPolicy,
    pub carries_phi: bool,
    pub input_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorAuditEvent {
    pub connector_name: String,
    pub logical_endpoint: String,
    pub tool_name: String,
    pub latency_ms: u64,
    pub status: String,
    pub input_sha256: String,
    pub output_sha256: String,
}

impl ConnectorAuditEvent {
    pub fn from_payloads(
        plan: &ConnectorCallPlan,
        latency_ms: u64,
        status: impl Into<String>,
        output_fingerprint_material: &[u8],
    ) -> Self {
        Self {
            connector_name: plan.connector_name.clone(),
            logical_endpoint: plan.logical_endpoint.clone(),
            tool_name: plan.tool_name.clone(),
            latency_ms,
            status: status.into(),
            input_sha256: plan.input_sha256.clone(),
            output_sha256: sha256_hex(output_fingerprint_material),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(phi_safe: bool) -> ConnectorRegistry {
        ConnectorRegistry::new(vec![ConnectorConfig {
            name: "guidelines-hub".into(),
            url: "https://guidelines.internal/search".into(),
            auth: ConnectorAuth::BearerEnv {
                env_var: "GUIDELINES_TOKEN".into(),
            },
            phi_safe,
            tools: vec!["search".into()],
            timeout_ms: 5_000,
            logging_policy: ConnectorLoggingPolicy::FingerprintOnly,
            enabled: true,
        }])
    }

    #[test]
    fn local_only_blocks_connector_calls() {
        let request = ConnectorCallRequest {
            connector_name: "guidelines-hub",
            tool_name: "search",
            data_boundary_mode: DataBoundaryMode::LocalOnly,
            carries_phi: false,
            user_confirmed_phi: false,
            input_fingerprint_material: b"query",
        };
        let err = registry(false).validate_call(&request).unwrap_err();
        assert!(matches!(err, Error::InvalidConfig(_)));
    }

    #[test]
    fn deid_cloud_blocks_phi_without_safe_declaration_and_confirmation() {
        let base = ConnectorCallRequest {
            connector_name: "guidelines-hub",
            tool_name: "search",
            data_boundary_mode: DataBoundaryMode::DeidCloud,
            carries_phi: true,
            user_confirmed_phi: false,
            input_fingerprint_material: b"phi",
        };
        assert!(registry(false).validate_call(&base).is_err());
        assert!(registry(true).validate_call(&base).is_err());
        let confirmed = ConnectorCallRequest {
            user_confirmed_phi: true,
            ..base
        };
        let plan = registry(true).validate_call(&confirmed).unwrap();
        assert!(plan.carries_phi);
        assert_eq!(plan.tool_name, "search");
    }

    #[test]
    fn audit_event_uses_hashes_only() {
        let request = ConnectorCallRequest {
            connector_name: "guidelines-hub",
            tool_name: "search",
            data_boundary_mode: DataBoundaryMode::DeidCloud,
            carries_phi: false,
            user_confirmed_phi: false,
            input_fingerprint_material: b"masked query",
        };
        let plan = registry(false).validate_call(&request).unwrap();
        let event = ConnectorAuditEvent::from_payloads(&plan, 12, "ok", b"response");
        assert_eq!(event.input_sha256, sha256_hex(b"masked query"));
        assert_eq!(event.output_sha256, sha256_hex(b"response"));
    }
}
