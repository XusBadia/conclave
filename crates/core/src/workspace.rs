//! Workspace lifecycle: create, list, load, delete.
//!
//! A workspace is a directory under `<data_dir>/workspaces/<id>/` containing:
//! - `workspace.toml` — the manifest serialised from [`Workspace`].
//! - `documents/` — copies of the user's ingested files (filled in Phase 1).
//! - `metadata.sqlite` — relational metadata (filled in Phase 1).
//! - `vectors.lance/` — the `LanceDB` store (filled in Phase 1).
//!
//! Only the manifest is created by this module; Phase 1 populates the rest.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use slug::slugify;

use crate::{Error, Result};

/// Maximum number of collision suffixes (`name-2`, `name-3`, …) tried before
/// giving up. Practically unreachable — guards against pathological input.
const MAX_COLLISION_SUFFIX: u32 = 1000;

/// In-memory representation of a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    /// Stable, filesystem-safe identifier. Derived from `name` via [`slug::slugify`].
    pub id: String,
    /// Human-friendly display name.
    pub name: String,
    /// Optional clinical specialty (e.g., `"cardiology"`, `"oncology"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specialty: Option<String>,
    /// Optional default language ISO code (e.g., `"es"`, `"en"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Timestamp when the workspace directory was first created.
    pub created_at: DateTime<Utc>,
}

impl Workspace {
    /// Path to a workspace's manifest file within its directory.
    pub fn manifest_path(root: &Path) -> PathBuf {
        root.join("workspace.toml")
    }
}

/// Lifecycle manager rooted at a `workspaces/` directory.
#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    root: PathBuf,
}

impl WorkspaceManager {
    /// Build a manager rooted at the given workspaces directory.
    ///
    /// The directory does not need to exist yet; [`Self::create`] will create
    /// it on demand.
    pub fn new(workspaces_root: impl Into<PathBuf>) -> Self {
        Self {
            root: workspaces_root.into(),
        }
    }

    /// Workspaces root directory this manager operates on.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create a new workspace and write its manifest. Returns the resolved
    /// in-memory representation with a stable, slug-based id.
    pub fn create(
        &self,
        name: &str,
        specialty: Option<String>,
        language: Option<String>,
    ) -> Result<Workspace> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(Error::invalid_config("workspace name must not be empty"));
        }
        let id = self.resolve_unique_id(trimmed)?;
        let dir = self.root.join(&id);
        std::fs::create_dir_all(&dir).map_err(|e| Error::io_at(&dir, e))?;

        let workspace = Workspace {
            id,
            name: trimmed.to_owned(),
            specialty,
            language,
            created_at: Utc::now(),
        };
        let raw = toml::to_string_pretty(&workspace)?;
        let manifest = Workspace::manifest_path(&dir);
        std::fs::write(&manifest, raw).map_err(|e| Error::io_at(&manifest, e))?;
        Ok(workspace)
    }

    /// List every workspace whose `workspace.toml` parses successfully.
    /// Directories without a manifest are silently skipped.
    pub fn list(&self) -> Result<Vec<Workspace>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(Error::io_at(&self.root, e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| Error::io_at(&self.root, e))?;
            let entry_path = entry.path();
            let is_dir = entry
                .file_type()
                .map_err(|e| Error::io_at(&entry_path, e))?
                .is_dir();
            if !is_dir {
                continue;
            }
            let manifest = Workspace::manifest_path(&entry_path);
            if !manifest.exists() {
                continue;
            }
            let raw = std::fs::read_to_string(&manifest).map_err(|e| Error::io_at(&manifest, e))?;
            let ws: Workspace = toml::from_str(&raw)?;
            out.push(ws);
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// Look up a workspace by exact id first, then by name.
    pub fn load(&self, id_or_name: &str) -> Result<Workspace> {
        let trimmed = id_or_name.trim();
        if trimmed.is_empty() {
            return Err(Error::invalid_config(
                "workspace identifier must not be empty",
            ));
        }

        let direct = Workspace::manifest_path(&self.root.join(trimmed));
        if direct.exists() {
            let raw = std::fs::read_to_string(&direct).map_err(|e| Error::io_at(&direct, e))?;
            let ws: Workspace = toml::from_str(&raw)?;
            return Ok(ws);
        }

        for ws in self.list()? {
            if ws.id == trimmed || ws.name == trimmed {
                return Ok(ws);
            }
        }
        Err(Error::WorkspaceNotFound(trimmed.to_owned()))
    }

    /// Remove a workspace's entire directory tree.
    pub fn delete(&self, id_or_name: &str) -> Result<()> {
        let ws = self.load(id_or_name)?;
        let dir = self.root.join(&ws.id);
        std::fs::remove_dir_all(&dir).map_err(|e| Error::io_at(&dir, e))
    }

    fn resolve_unique_id(&self, name: &str) -> Result<String> {
        let base = slugify(name);
        if base.is_empty() {
            return Err(Error::invalid_config(
                "workspace name slugifies to empty string",
            ));
        }
        if !self.root.join(&base).exists() {
            return Ok(base);
        }
        for n in 2..=MAX_COLLISION_SUFFIX {
            let candidate = format!("{base}-{n}");
            if !self.root.join(&candidate).exists() {
                return Ok(candidate);
            }
        }
        Err(Error::WorkspaceExists(base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_in(tmp: &tempfile::TempDir) -> WorkspaceManager {
        WorkspaceManager::new(tmp.path().to_path_buf())
    }

    #[test]
    fn create_then_load_by_id_and_name() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = manager_in(&tmp);

        let ws = mgr
            .create(
                "Cardiología Adultos",
                Some("cardiology".into()),
                Some("es".into()),
            )
            .unwrap();
        assert_eq!(ws.id, "cardiologia-adultos");
        assert_eq!(ws.name, "Cardiología Adultos");
        assert_eq!(ws.specialty.as_deref(), Some("cardiology"));
        assert_eq!(ws.language.as_deref(), Some("es"));

        let by_id = mgr.load(&ws.id).unwrap();
        assert_eq!(by_id, ws);

        let by_name = mgr.load("Cardiología Adultos").unwrap();
        assert_eq!(by_name.id, ws.id);
    }

    #[test]
    fn create_appends_suffix_on_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = manager_in(&tmp);
        let a = mgr.create("Oncology", None, None).unwrap();
        let b = mgr.create("Oncology", None, None).unwrap();
        assert_eq!(a.id, "oncology");
        assert_eq!(b.id, "oncology-2");
    }

    #[test]
    fn list_skips_directories_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("not-a-workspace")).unwrap();
        let mgr = manager_in(&tmp);
        mgr.create("Foo", None, None).unwrap();
        let listed = mgr.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Foo");
    }

    #[test]
    fn list_on_missing_root_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = WorkspaceManager::new(tmp.path().join("not-yet-created"));
        let listed = mgr.list().unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn delete_removes_directory_and_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = manager_in(&tmp);
        let ws = mgr.create("Tumor Board", None, None).unwrap();
        assert!(tmp.path().join(&ws.id).exists());
        mgr.delete(&ws.id).unwrap();
        assert!(!tmp.path().join(&ws.id).exists());
        assert!(mgr.list().unwrap().is_empty());
    }

    #[test]
    fn empty_name_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = manager_in(&tmp);
        assert!(mgr.create("   ", None, None).is_err());
        assert!(mgr.create("", None, None).is_err());
    }

    #[test]
    fn unknown_workspace_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = manager_in(&tmp);
        let err = mgr.load("ghost").unwrap_err();
        assert!(matches!(err, Error::WorkspaceNotFound(_)));
    }

    #[test]
    fn manifest_round_trips_through_toml() {
        let original = Workspace {
            id: "x".into(),
            name: "x".into(),
            specialty: Some("cardio".into()),
            language: Some("es".into()),
            created_at: Utc::now(),
        };
        let raw = toml::to_string_pretty(&original).unwrap();
        let parsed: Workspace = toml::from_str(&raw).unwrap();
        assert_eq!(parsed, original);
    }
}
