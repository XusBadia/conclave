//! OS-standard application paths resolved via the `directories` crate.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::{Error, Result, APP_NAME, APP_ORGANIZATION, APP_QUALIFIER};

/// Resolved application directories.
///
/// Paths follow each operating system's conventions:
/// - Linux: XDG base directory specification
/// - macOS: `~/Library/Application Support`, `~/Library/Caches`, etc.
/// - Windows: `%APPDATA%`, `%LOCALAPPDATA%`
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct Paths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl Paths {
    /// Resolve application paths from the host operating system.
    pub fn resolve() -> Result<Self> {
        let proj = ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .ok_or(Error::MissingAppDirs)?;
        Ok(Self {
            config_dir: proj.config_dir().to_path_buf(),
            data_dir: proj.data_dir().to_path_buf(),
            cache_dir: proj.cache_dir().to_path_buf(),
        })
    }

    /// Construct an explicit [`Paths`] rooted at `root`.
    ///
    /// Useful for tests and for the `--workspace-root` CLI flag that pins all
    /// state under a single directory.
    pub fn rooted_at(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        }
    }

    /// Directory holding user configuration files.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Directory holding durable application data (knowledge base, indices).
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Directory holding regenerable cache data.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Path to the main `conclave.toml` configuration file.
    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("conclave.toml")
    }

    /// Directory containing every workspace, one subdirectory per id.
    pub fn workspaces_dir(&self) -> PathBuf {
        self.data_dir.join("workspaces")
    }

    /// Directory for an individual workspace identified by `id`.
    pub fn workspace_dir(&self, id: &str) -> PathBuf {
        self.workspaces_dir().join(id)
    }

    /// Create every directory in this set if it does not already exist.
    pub fn ensure_exists(&self) -> Result<()> {
        for dir in [&self.config_dir, &self.data_dir, &self.cache_dir] {
            std::fs::create_dir_all(dir).map_err(|e| Error::io_at(dir, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooted_at_lays_out_expected_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(tmp.path());

        assert_eq!(paths.config_dir(), tmp.path().join("config"));
        assert_eq!(paths.data_dir(), tmp.path().join("data"));
        assert_eq!(paths.cache_dir(), tmp.path().join("cache"));
        assert_eq!(paths.config_file(), tmp.path().join("config/conclave.toml"));
    }

    #[test]
    fn ensure_exists_creates_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(tmp.path());

        paths.ensure_exists().unwrap();

        assert!(paths.config_dir().is_dir());
        assert!(paths.data_dir().is_dir());
        assert!(paths.cache_dir().is_dir());
    }
}
