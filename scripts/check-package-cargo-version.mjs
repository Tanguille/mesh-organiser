import { readFileSync } from "node:fs";

const pkg = JSON.parse(readFileSync("package.json", "utf-8"));
const cargo = readFileSync("Cargo.toml", "utf-8");
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
