#!/usr/bin/env bash
set -euo pipefail

OCCT_ROOT="$RUNNER_TEMP/occt"
echo "OCCT_ROOT=$OCCT_ROOT" >> "$GITHUB_ENV"
cadrum_manifest=$(cargo metadata --locked --format-version 1 | jq -er '.packages[] | select(.name == "cadrum") | .manifest_path')
cadrum_build_script="${cadrum_manifest%/*}/build.rs"
occt_version=$(sed -n 's/^const OCCT_VERSION: &str = "\(.*\)";/\1/p' "$cadrum_build_script")
if [ -z "$occt_version" ]; then
  echo "::error::Could not read OCCT_VERSION from $cadrum_build_script; cadrum changed its build script."
  exit 1
fi
mkdir -p "$OCCT_ROOT"
curl --fail --location --retry 3 "https://codeload.github.com/Open-Cascade-SAS/OCCT/tar.gz/refs/tags/${occt_version}" | tar -xz -C "$OCCT_ROOT"
