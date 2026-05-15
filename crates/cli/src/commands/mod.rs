//! Subcommand modules for `conclave-cli`.
//!
//! Each subcommand exposes an `Args` struct (used by `clap`) and a `run`
//! function. They share a [`CommandContext`] carrying the resolved paths and
//! parsed configuration.

use conclave_core::{paths::Paths, Config};

pub(crate) mod ingest;
pub(crate) mod providers;
pub(crate) mod verdict;
pub(crate) mod workspace;

/// Runtime context handed to each subcommand.
#[derive(Debug)]
pub(crate) struct CommandContext {
    /// Resolved application paths (config / data / cache).
    pub(crate) paths: Paths,
    /// Parsed configuration.
    pub(crate) config: Config,
}
