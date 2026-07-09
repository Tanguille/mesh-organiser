/**
 * Invariants from `.github/RELEASE.md`: single app version, Tauri config consistency.
 * Run in CI (Svelte, publish) via `node scripts/check-package-cargo-version.mjs`.
 */
import { readFileSync } from "node:fs";

const pkg = JSON.parse(readFileSync("package.json", "utf-8"));
const cargo = readFileSync("Cargo.toml", "utf-8");
const tauriPath = "src-tauri/tauri.conf.json";
const tauri = JSON.parse(readFileSync(tauriPath, "utf-8"));
const lines = cargo.split(/\r?\n/);

let inWorkspacePackage = false;
let cargoVersion = null;

for (const line of lines) {
  if (line.trim() === "[workspace.package]") {
    inWorkspacePackage = true;
    continue;
  }

  if (inWorkspacePackage && /^\[/.test(line)) {
    break;
  }

  if (inWorkspacePackage) {
    const m = line.match(/^version\s*=\s*"([^"]+)"/);
    if (m) {
      cargoVersion = m[1];
      break;
    }
  }
}

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
