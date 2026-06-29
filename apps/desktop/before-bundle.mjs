// Tauri `beforeBundleCommand`. On macOS, rewrite the libswift_Concurrency
// dylib path in the built binary so the bundle links against the system
// Swift runtime instead of an @rpath that isn't present at runtime. No-op on
// Windows and Linux.
//
// This used to be an inline bash one-liner in tauri.conf.json, but Tauri runs
// `beforeBundleCommand` through the platform's default shell — cmd.exe on
// Windows — which can't parse bash syntax and failed the Windows bundle.
// `node before-bundle.mjs` is parsed identically by cmd, sh and bash.
//
// Runs with cwd = apps/desktop (the Tauri projectPath), hence the
// `../../target` paths into the workspace target dir.
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";

if (process.platform !== "darwin") {
  process.exit(0);
}

const bins = [
  "../../target/release/conclave-desktop",
  "../../target/universal-apple-darwin/release/conclave-desktop",
  "../../target/aarch64-apple-darwin/release/conclave-desktop",
  "../../target/x86_64-apple-darwin/release/conclave-desktop",
];

for (const bin of bins) {
  if (!existsSync(bin)) continue;
  try {
    execFileSync("install_name_tool", [
      "-change",
      "@rpath/libswift_Concurrency.dylib",
      "/usr/lib/swift/libswift_Concurrency.dylib",
      bin,
    ]);
  } catch {
    // Best-effort, mirrors the original `|| true`: not every binary needs
    // the rewrite, and a missing/already-patched dylib path must not fail
    // the bundle.
  }
}
