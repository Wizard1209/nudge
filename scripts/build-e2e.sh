#!/usr/bin/env bash
#
# Build the e2e WASM bundle with BOTH the popup (index.html) and the settings
# UI (settings.html) co-located in e2e/dist. Both pages reference the same
# wasm/js artefacts; the shared wasm_entry::start dispatches between
# NudgeApp and SettingsApp based on the URL (path or ?settings query).
#
# Why two trunk invocations:
#   Trunk wipes --dist on every build, so a single `trunk build e2e/index.html`
#   leaves dist with only the popup HTML — `settings.html` 404s and the
#   e2e/tests/settings.test.ts suite times out waiting for the canvas.
#   We build the settings page into a side dist first, then build index.html
#   into the real dist (whose wasm/js the settings page reuses by hash since
#   the underlying Rust crate is identical), then drop the settings HTML in
#   alongside it.
#
# Usage:
#   scripts/build-e2e.sh
#
# After this script, run the test server + vitest per e2e/CLAUDE.md.
set -euo pipefail

cd "$(dirname "$0")/.."

# Settings page first into a side dist — its dist/index.html is the settings
# HTML we'll later rename into the real dist.
trunk build e2e/settings.html --dist e2e/dist-settings

# Then the popup into the real dist — this overwrites e2e/dist with the
# popup's index.html (plus the wasm/js, which the settings HTML also targets
# because both link the same Cargo target).
trunk build e2e/index.html --dist e2e/dist

# Park the settings HTML in dist under its expected name. Both HTMLs reference
# the same hashed wasm/js, so no further file copying is needed.
cp e2e/dist-settings/index.html e2e/dist/settings.html
rm -rf e2e/dist-settings
