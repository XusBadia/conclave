#!/usr/bin/env node
// Regenerate the Tauri app icon bundle from src-tauri/icons/source.svg.
//
// Pipeline:
//   1. Rasterize source.svg → _source-1024.png via @resvg/resvg-js.
//   2. Invoke `pnpm tauri icon` against the PNG, which produces every
//      platform-specific format (.icns, .ico, sized PNGs, Windows Store
//      tiles) into src-tauri/icons/.
//   3. Delete the throwaway _source-1024.png and mobile byproducts.
//
// Run with `pnpm --dir apps/desktop run icons` (see package.json scripts).

import { Resvg } from "@resvg/resvg-js";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, rmSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const desktopDir = resolve(__dirname, "..");
const iconsDir = resolve(desktopDir, "src-tauri", "icons");
const sourceSvg = resolve(iconsDir, "source.svg");
const masterPng = resolve(iconsDir, "_source-1024.png");

console.log(`[icons] rasterizing ${sourceSvg} → ${masterPng}`);
const svg = readFileSync(sourceSvg);
const resvg = new Resvg(svg, {
  fitTo: { mode: "width", value: 1024 },
  background: "rgba(0, 0, 0, 0)",
});
writeFileSync(masterPng, resvg.render().asPng());

console.log(`[icons] running tauri icon`);
execFileSync(
  "pnpm",
  ["tauri", "icon", masterPng, "--output", iconsDir],
  { cwd: desktopDir, stdio: "inherit" },
);

console.log(`[icons] cleaning up ${masterPng}`);
rmSync(masterPng);
for (const mobileDir of ["android", "ios"]) {
  rmSync(resolve(iconsDir, mobileDir), { recursive: true, force: true });
}

console.log(`[icons] done`);
