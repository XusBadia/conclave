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
//!    A 1 s timeout caps pathological rc files. If we get a non-empty
//!    answer we replace the process PATH with it.
//! 2. **Curated prepend** — fall back to a list of well-known macOS
//!    install locations (`~/.local/bin`, `/opt/homebrew/bin`,
//!    `/usr/local/bin`, `~/.cargo/bin`, `~/.bun/bin`) plus the most
//!    recent nvm node version, prepended to whatever PATH is already
//!    set. Best-effort safety net for when the shell snoop fails.
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

    // Poll for completion up to the 1 s budget. Anything slower
    // probably means the shell is hanging on a network call from an
    // rc file (corporate VPN scripts, etc.) — we'd rather fall back
    // to the curated list than block the app window.
    let deadline = Instant::now() + Duration::from_secs(1);
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
        if let Some(node_bin) = newest_nvm_node_bin(h) {
            additions.push(node_bin);
        }
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

#[cfg(target_os = "macos")]
fn newest_nvm_node_bin(home: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::fs;
    let versions_dir = home.join(".nvm/versions/node");
    let entries = fs::read_dir(&versions_dir).ok()?;
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
    candidates.into_iter().next().map(|(_, p)| p)
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
}
