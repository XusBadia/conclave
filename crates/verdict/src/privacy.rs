//! Privacy posture and data-boundary helpers shared by CLI, Tauri, and the
//! verdict pipelines.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How much raw clinical text is retained locally for a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawTextRetention {
    /// Legacy row created before Conclave stopped retaining raw narratives by
    /// default. We keep this label so old data is visible but not silently
    /// destroyed.
    LegacyRetained,
    /// Raw text is temporarily retained while a draft is still editable.
    TemporaryDraft,
    /// Raw text was explicitly retained by the user on this machine.
    ExplicitRetained,
    /// Raw text was never stored or has been purged.
    Discarded,
}

impl RawTextRetention {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::LegacyRetained => "legacy_retained",
            Self::TemporaryDraft => "temporary_draft",
            Self::ExplicitRetained => "explicit_retained",
            Self::Discarded => "discarded",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "temporary_draft" => Self::TemporaryDraft,
            "explicit_retained" => Self::ExplicitRetained,
            "discarded" => Self::Discarded,
            _ => Self::LegacyRetained,
        }
    }
}

/// Session-level execution boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataBoundaryMode {
    /// No network provider, remote search, or cloud vision.
    LocalOnly,
    /// Default: de-identified text may leave the machine; raw PHI payloads do
    /// not.
    #[default]
    DeidCloud,
    /// Raw PHI payloads such as images may leave the machine after explicit
    /// per-run consent.
    ExplicitPhi,
}

impl DataBoundaryMode {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::DeidCloud => "deid_cloud",
            Self::ExplicitPhi => "explicit_phi",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "local_only" => Self::LocalOnly,
            "explicit_phi" => Self::ExplicitPhi,
            _ => Self::DeidCloud,
        }
    }
}

/// How much prompt/output payload an audit run stores.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditPayloadMode {
    None,
    #[default]
    Fingerprint,
    Preview,
    Payload,
}

impl AuditPayloadMode {
    pub const fn as_db_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fingerprint => "fingerprint",
            Self::Preview => "preview",
            Self::Payload => "payload",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "preview" => Self::Preview,
            "payload" => Self::Payload,
            _ => Self::Fingerprint,
        }
    }
}

/// Stable SHA-256 hex digest.
pub fn sha256_hex(input: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_ref());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}
