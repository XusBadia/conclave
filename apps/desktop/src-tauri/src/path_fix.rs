//! Restores a usable `$PATH` for the running process when the app was
//! launched from Finder/Dock.
//!
//! macOS GUI apps inherit the bare `launchd` environment, which is
//! roughly `/usr/bin:/bin:/usr/sbin:/sbin` plus a few cryptex entries.
//! That set does not include the directories where most developer CLIs
//! live (`~/.local/bin`, Homebrew, nvm, Cargo, Bun, …), so any
//! `which::which("claude")` style lookup from inside the bundled app
//! comes back empty even when the binary exists and is on the user's
//! shell PATH. The CLI provider tiles in Settings then render as
//! "not installed" and the user is stuck.
//!
//! The fix has two strategies, applied in order:
//!
//! 1. **Shell snoop** — spawn the user's login shell with `-ilc 'printenv
//!    PATH'`. This loads `.zshrc` / `.zprofile` / `.bashrc` the same way
//!    a Terminal window would and prints the resulting PATH on stdout.
//!    A 2 s timeout caps pathological rc files. If we get a non-empty
//!    answer we replace the process PATH with it.
//! 2. **Curated prepend** — fall back to a list of well-known macOS
//!    install locations (`~/.local/bin`, `/opt/homebrew/bin`,
//!    `/usr/local/bin`, `~/.cargo/bin`, `~/.bun/bin`) plus *every* nvm
//!    node version's `bin`, prepended to whatever PATH is already set.
//!    Best-effort safety net for when the shell snoop fails.
//!
//! `augment_path_for_gui` must run **before** any provider construction,
//! because `crates/providers/src/{claude_cli,codex_cli}.rs` cache the
//! `which::which` result in a process-wide `OnceLock` (now `RwLock`,
//! but still cached until explicitly invalidated). Calling it as the
//! first line of `lib::run()` satisfies that constraint.
//!
//! Non-macOS builds get a no-op — Linux and Windows GUI launchers
//! either pass the user PATH through (Linux desktop entries, depending
//! on launcher) or have their own conventions Conclave doesn't ship for
//! today.

#[cfg(target_os = "macos")]
pub fn augment_path_for_gui() {
    let original = std::env::var("PATH").unwrap_or_default();

    if let Some(snooped) = snoop_shell_path() {
        let snooped = snooped.trim();
        if !snooped.is_empty() && snooped != original {
            tracing::info!(
                target: "conclave_desktop::path_fix",
                "augmented PATH from login shell snoop"
            );
            // SAFETY: set_var is only `unsafe` on the 2024 edition; the
            // crate is on 2021 edition so the plain call is fine. We
            // wrap regardless so a future edition bump keeps building.
            std::env::set_var("PATH", snooped);
            return;
        }
    }

    let merged = prepend_well_known(&original);
    if merged != original {
        tracing::info!(
            target: "conclave_desktop::path_fix",
            "augmented PATH with curated fallback entries"
        );
        std::env::set_var("PATH", merged);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn augment_path_for_gui() {}

#[cfg(target_os = "macos")]
fn snoop_shell_path() -> Option<String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    // `-i` (interactive) is required so `.zshrc` is sourced; `-l`
    // (login) so `.zprofile` runs too; `-c` runs the printenv command
    // and exits. Some users put `tput` calls in their rc files that
    // bail on a non-tty stdout — printenv writes raw bytes so we're
    // immune to that.
    let mut child = Command::new(&shell)
        .args(["-ilc", "printenv PATH"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Poll for completion up to the budget below. A heavy-but-honest
    // interactive zsh (nvm + pnpm + rbenv + assorted rc tooling) can
    // legitimately take well over a second to finish sourcing, so the
    // old 1 s cap kicked such users onto the curated fallback — which
    // then had to guess their nvm layout and got it wrong. 2 s covers a
    // real shell while still bounding a genuinely hung rc file (a
    // corporate VPN script blocking on the network) so startup can't
    // wedge.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let out = child.wait_with_output().ok()?;
                let s = String::from_utf8(out.stdout).ok()?;
                return Some(s);
            }
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn prepend_well_known(current: &str) -> String {
    use std::path::PathBuf;

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut additions: Vec<PathBuf> = Vec::new();

    if let Some(h) = &home {
        additions.push(h.join(".local/bin"));
    }
    additions.push(PathBuf::from("/opt/homebrew/bin"));
    additions.push(PathBuf::from("/opt/homebrew/sbin"));
    additions.push(PathBuf::from("/usr/local/bin"));
    if let Some(h) = &home {
        additions.push(h.join(".cargo/bin"));
        additions.push(h.join(".bun/bin"));
        // A globally-installed CLI (`npm i -g codex`) lives under exactly
        // one nvm node version, and which one is invisible from out here.
        // Add every version's `bin` so `which` can find the tool wherever
        // it actually sits, rather than guessing a single one.
        additions.extend(all_nvm_node_bins(h));
    }

    // Only add entries that (a) exist on disk and (b) aren't already in
    // PATH. Avoids cosmetic noise and keeps the order deterministic.
    let existing: Vec<&str> = current.split(':').filter(|s| !s.is_empty()).collect();
    let mut prepend: Vec<String> = Vec::new();
    for p in additions {
        let s = p.to_string_lossy().into_owned();
        if !p.is_dir() {
            continue;
        }
        if existing.iter().any(|e| *e == s) {
            continue;
        }
        if prepend.contains(&s) {
            continue;
        }
        prepend.push(s);
    }

    if prepend.is_empty() {
        return current.to_owned();
    }
    if current.is_empty() {
        return prepend.join(":");
    }
    format!("{}:{}", prepend.join(":"), current)
}

/// Every nvm-managed node version's `bin` directory, ordered newest
/// mtime first.
///
/// A globally-installed CLI (`npm i -g codex`) lands in exactly one node
/// version's `bin`, and which one is invisible from outside the shell:
/// nvm rewrites PATH at shell-init time, which is precisely the signal
/// we've lost when the snoop fails. An earlier version of this code
/// picked a single version by directory mtime and routinely guessed
/// wrong — the most-recently-*touched* node version is rarely the one a
/// given tool was installed under (e.g. `codex` under v25.2.0 while a
/// later `npm i -g` under v22 bumped v22's mtime). So we surface every
/// version's `bin` and let `which::which` find the tool wherever it
/// actually lives. Newest-first keeps `node` / `npm` resolution biased
/// toward the most recent install when several versions provide them.
#[cfg(target_os = "macos")]
fn all_nvm_node_bins(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    use std::fs;
    let versions_dir = home.join(".nvm/versions/node");
    let Ok(entries) = fs::read_dir(&versions_dir) else {
        return Vec::new();
    };
    let mut candidates: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let bin = entry.path().join("bin");
        if !bin.is_dir() {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        candidates.push((mtime, bin));
    }
    candidates.sort_by_key(|c| std::cmp::Reverse(c.0));
    candidates.into_iter().map(|(_, p)| p).collect()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn prepend_skips_existing_entries() {
        // Use a path that definitely exists on every macOS box: /tmp.
        let cur = "/tmp:/usr/bin";
        let out = prepend_well_known(cur);
        // /tmp must not be duplicated; output starts with whatever
        // well-known entries are present on this host.
        let count = out.matches("/tmp").count();
        assert_eq!(
            count, 1,
            "PATH should not duplicate existing entries: {out}"
        );
    }

    #[test]
    fn prepend_no_op_when_no_dirs_to_add() {
        // Seed with every well-known dir present so additions is empty.
        // We can't guarantee any single path exists on every dev box, so
        // we just assert the contract: the returned string is the same
        // string when there's nothing to add.
        let saturated = format!(
            "{}/opt/homebrew/bin:/usr/local/bin",
            std::env::var("HOME")
                .map(|h| format!("{h}/.local/bin:{h}/.cargo/bin:{h}/.bun/bin:"))
                .unwrap_or_default()
        );
        let out = prepend_well_known(&saturated);
        // The output may add things that ARE present and not yet listed,
        // but it must never DROP an existing entry.
        for entry in saturated.split(':').filter(|s| !s.is_empty()) {
            assert!(out.contains(entry), "lost entry {entry} in {out}");
        }
    }

    #[test]
    fn all_nvm_bins_returns_every_version_with_a_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join(".nvm/versions/node");
        for v in ["v20.19.6", "v22.21.1", "v25.2.0"] {
            std::fs::create_dir_all(base.join(v).join("bin")).unwrap();
        }
        // A stray file (not a dir) and a version dir missing `bin/` must
        // both be ignored — regression guard for the `is_dir` filter.
        std::fs::write(base.join("not-a-version"), "x").unwrap();
        std::fs::create_dir_all(base.join("v18.0.0")).unwrap();

        let bins = all_nvm_node_bins(tmp.path());
        assert_eq!(
            bins.len(),
            3,
            "only versions with a bin/ dir count: {bins:?}"
        );
        // The regression we actually fixed: a version that is NOT the
        // newest by mtime (here, where `codex` would live) is still
        // surfaced instead of being dropped in favour of one bin.
        for v in ["v20.19.6", "v22.21.1", "v25.2.0"] {
            assert!(
                bins.iter().any(|p| p.ends_with(format!("{v}/bin"))),
                "missing {v} in {bins:?}"
            );
        }
    }

    #[test]
    fn all_nvm_bins_empty_without_nvm_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(all_nvm_node_bins(tmp.path()).is_empty());
    }
}
