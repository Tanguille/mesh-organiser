/**
 * Invariants from `.github/RELEASE.md`: single app version, Tauri config consistency.
 * Run in CI (Svelte, publish) via `node scripts/check-package-cargo-version.mjs`.
 */
import { readFileSync } from "node:fs";

const pkg = JSON.parse(readFileSync("package.json", "utf-8"));
const cargo = readFileSync("Cargo.toml", "utf-8");
const tauriPath = "src-tauri/tauri.conf.json";
const tauri = JSON.parse(readFileSync(tauriPath, "utf-8"));
// Splitting on line-start `[` isolates each section body, so a `[` inside a
// value cannot bleed the match across sections.
const cargoVersion = cargo
  .split(/^\[/m)
  .find((section) => section.startsWith("workspace.package]"))
  ?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

if (cargoVersion == null) {
  console.error(
    "Could not find version under [workspace.package] in Cargo.toml",
  );
  process.exit(1);
}

if (pkg.version !== cargoVersion) {
  console.error(
    `Version mismatch: package.json "${pkg.version}" !== Cargo.toml [workspace.package] "${cargoVersion}"`,
  );
  process.exit(1);
}

if (tauri.version !== "../package.json") {
  console.error(
    `${tauriPath}: "version" must be "../package.json" so the app version matches package.json (see .github/RELEASE.md). Got: ${JSON.stringify(tauri.version)}`,
  );
  process.exit(1);
}

console.log(
  `Release invariants OK: app version ${pkg.version} (package.json, Cargo workspace, Tauri via ${tauriPath})`,
);
