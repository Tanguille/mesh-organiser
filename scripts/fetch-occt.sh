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

# Mirror cadrum's source discovery so an incomplete extraction fails in this step.
if ! find "$OCCT_ROOT" -mindepth 1 -maxdepth 1 -type d -name 'OCCT*' -print -quit | grep -q . ||
  ! find "$OCCT_ROOT" -maxdepth 2 -name 'OCCT_LGPL_EXCEPTION.txt' -print -quit | grep -q .; then
  echo "::error::OCCT extraction under $OCCT_ROOT is missing an OCCT* source directory or OCCT_LGPL_EXCEPTION.txt within two levels; cadrum would download OCCT itself."
  exit 1
fi
