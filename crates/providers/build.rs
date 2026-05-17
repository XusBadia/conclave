// Compile the Apple Intelligence Swift bridge into a static library and
// emit the link directives needed so the final Rust binary can resolve
// `FoundationModels` symbols against the system framework.
//
// On non-macOS hosts this script is a no-op — the Rust side substitutes a
// stub implementation that always reports `Unavailable`, so the workspace
// keeps building on Linux/Windows CI.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Declare the cfg so downstream check-cfg analysis stays silent;
    // we set it conditionally below when the Swift toolchain isn't
    // available and the Rust side needs to fall back to the stub.
    println!("cargo::rustc-check-cfg=cfg(apple_intel_stub)");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "macos" {
        return;
    }

    let swift_src = PathBuf::from("swift/AppleIntelligenceBridge.swift");
    println!("cargo:rerun-if-changed={}", swift_src.display());
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let staticlib = out_dir.join("libapple_intel_bridge.a");

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let swift_target = match target_arch.as_str() {
        "aarch64" => "arm64-apple-macos26.0",
        "x86_64" => "x86_64-apple-macos26.0",
        other => {
            // Architectures we don't know about — leave the Swift code
            // uncompiled and let the Rust stub take over. This keeps
            // `cargo check` and IDE tooling working on exotic hosts.
            println!(
                "cargo:warning=skipping Apple Intelligence Swift bridge for unsupported arch `{other}`"
            );
            return;
        }
    };

    let status = Command::new("xcrun")
        .args(["-sdk", "macosx", "swiftc"])
        .args(["-target", swift_target])
        .args(["-emit-library", "-static"])
        .args(["-module-name", "AppleIntelligenceBridge"])
        .args(["-parse-as-library", "-O"])
        .arg(&swift_src)
        .arg("-o")
        .arg(&staticlib)
        .status();

    let status = match status {
        Ok(s) => s,
        Err(e) => {
            // No Xcode toolchain installed — fall back to the stub so a
            // fresh checkout can still build the rest of the workspace.
            // The runtime path will report `Unavailable` cleanly.
            println!(
                "cargo:warning=xcrun unavailable, building Apple Intelligence stub only ({e})"
            );
            println!("cargo:rustc-cfg=apple_intel_stub");
            return;
        }
    };

    if !status.success() {
        println!(
            "cargo:warning=swiftc failed for the Apple Intelligence bridge — falling back to stub"
        );
        println!("cargo:rustc-cfg=apple_intel_stub");
        return;
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=apple_intel_bridge");

    // System frameworks the Swift code calls at runtime.
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=FoundationModels");

    // The bridge uses `Task`, which pulls in `libswift_Concurrency.dylib`
    // at runtime. The rpaths that resolve it live in the workspace
    // `.cargo/config.toml` because `cargo:rustc-link-arg` does not
    // propagate to dependent binaries — putting them in build.rs would
    // only fix the providers crate's own test binary, not the Tauri
    // backend or the CLI.
}
