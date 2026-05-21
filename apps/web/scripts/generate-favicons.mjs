/**
 * Derive favicon.ico, apple-touch-icon.png, icon-192.png, icon-512.png from mark.svg.
 * Runs as `prebuild` so the static export always ships fresh icons matching the brand mark.
 *
 * Idempotent: skips work if outputs are newer than the source SVG.
 */
import { Resvg } from "@resvg/resvg-js";
import { readFileSync, writeFileSync, statSync, existsSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, "..");
const SRC = resolve(ROOT, "public/mark.svg");
const PUBLIC = resolve(ROOT, "public");

const TARGETS = [
  { file: "apple-touch-icon.png", size: 180, padding: 24, bg: "#faf9f8" },
  { file: "icon-192.png", size: 192, padding: 28, bg: "transparent" },
  { file: "icon-512.png", size: 512, padding: 72, bg: "transparent" },
  { file: "favicon-32.png", size: 32, padding: 4, bg: "transparent" },
  { file: "favicon-16.png", size: 16, padding: 2, bg: "transparent" },
];

const sourceSvg = readFileSync(SRC, "utf-8");
const sourceMtime = statSync(SRC).mtimeMs;

function isStale(out) {
  if (!existsSync(out)) return true;
  return statSync(out).mtimeMs < sourceMtime;
}

function wrapWithBackground(svg, bg) {
  if (bg === "transparent") return svg;
  return svg.replace(
    /<svg([^>]*)>/,
    `<svg$1><rect width="100%" height="100%" fill="${bg}" />`,
  );
}

function renderPng({ size, padding, bg }) {
  const innerSize = size - padding * 2;
  const paddedSvg = sourceSvg.replace(
    /viewBox="0 0 64 64"/,
    `viewBox="0 0 64 64" width="${innerSize}" height="${innerSize}"`,
  );
  const wrapped = `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">${
    bg !== "transparent" ? `<rect width="100%" height="100%" fill="${bg}"/>` : ""
  }<g transform="translate(${padding} ${padding})">${paddedSvg.replace(
    /<svg[^>]*>|<\/svg>/g,
    "",
  )}</g></svg>`;
  const resvg = new Resvg(wrapped, {
    fitTo: { mode: "width", value: size },
    background: bg === "transparent" ? undefined : bg,
  });
  return resvg.render().asPng();
}

let written = 0;
for (const target of TARGETS) {
  const outPath = resolve(PUBLIC, target.file);
  if (!isStale(outPath)) continue;
  const png = renderPng(target);
  writeFileSync(outPath, png);
  written++;
  console.log(`  · ${target.file} (${target.size}×${target.size})`);
}

// favicon.ico is built by concatenating 16/32 PNGs into a minimal ICO container.
// resvg doesn't write ICO directly; we emit a single 32×32 PNG bytes inside an ICO header.
const icoPath = resolve(PUBLIC, "favicon.ico");
if (isStale(icoPath)) {
  const png32 = readFileSync(resolve(PUBLIC, "favicon-32.png"));
  // Minimal ICO with one 32×32 PNG entry
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: ICO
  header.writeUInt16LE(1, 4); // count
  const entry = Buffer.alloc(16);
  entry.writeUInt8(32, 0); // width
  entry.writeUInt8(32, 1); // height
  entry.writeUInt8(0, 2); // colors
  entry.writeUInt8(0, 3); // reserved
  entry.writeUInt16LE(1, 4); // planes
  entry.writeUInt16LE(32, 6); // bit depth
  entry.writeUInt32LE(png32.length, 8); // size
  entry.writeUInt32LE(6 + 16, 12); // offset
  writeFileSync(icoPath, Buffer.concat([header, entry, png32]));
  written++;
  console.log("  · favicon.ico (32×32 PNG)");
}

if (written === 0) {
  console.log("favicons: up to date");
} else {
  console.log(`favicons: ${written} written`);
}
