#!/usr/bin/env bash
#
# Assemble Nudge.app from a release build.
#
# A bundle is not cosmetic packaging on macOS — three things depend on it:
#
#   * `SMAppService.mainApp` registers a *bundle*, not an executable, so
#     launch-at-login cannot be implemented against a bare binary the way the
#     Windows build writes an exe path into HKCU\...\Run;
#   * a bare Mach-O launched from Finder opens a Terminal window alongside it;
#   * `LSUIElement` (no Dock icon, no Cmd-Tab entry) states in the bundle what
#     the binary already asks AppKit for at launch, so Launch Services agrees
#     with the running app from the moment it is double-clicked.
#
# Usage:
#   scripts/build-macos-app.sh                 # menu-bar only (LSUIElement=1)
#   AGENT_MODE=0 scripts/build-macos-app.sh    # keep the Dock icon, for debugging
#
# Requires: rustup target aarch64-apple-darwin, and the Command Line Tools for
# codesign/plutil (full Xcode is not needed).

set -euo pipefail

cd "$(dirname "$0")/.."

APP_NAME="Nudge"
BUNDLE_ID="com.wizard1209.nudge"
TARGET="aarch64-apple-darwin"
OUT_DIR="target/macos"
APP="${OUT_DIR}/${APP_NAME}.app"

# Single source of truth for the version: the same Cargo.toml the release
# workflow already checks against the git tag.
VERSION="$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)"

# LSUIElement hides the app from the Dock and Cmd-Tab. On by default now that
# the menu-bar status item exists: without it a user would have no way to quit
# an app that has neither a Dock icon (the binary sets
# `NSApplicationActivationPolicy::Accessory` itself, whatever this plist says)
# nor a window between nudges. The status item's Quit entry is that way out.
#
# AGENT_MODE=0 withholds the key. Useful only for debugging a launch problem,
# where a Dock icon is somewhere to force-quit from — it does NOT put one back,
# since the activation policy is the app's own call.
if [[ "${AGENT_MODE:-1}" == "1" ]]; then
  LSUIELEMENT="<key>LSUIElement</key><true/>"
else
  LSUIELEMENT="<!-- LSUIElement withheld via AGENT_MODE=0 -->"
fi

# UNIVERSAL=1 adds the Intel slice. Off by default because the second target
# doubles build time and most machines running this are Apple Silicon; CI turns
# it on so the published DMG opens on both.
TARGETS=("${TARGET}")
[[ "${UNIVERSAL:-0}" == "1" ]] && TARGETS+=("x86_64-apple-darwin")

for t in "${TARGETS[@]}"; do
  echo "==> Building ${APP_NAME} ${VERSION} for ${t}"
  rustup target add "${t}" >/dev/null 2>&1 || true
  cargo build --release --bin nudge --target "${t}"
done

echo "==> Assembling bundle"
rm -rf "${APP}"
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"

if [[ ${#TARGETS[@]} -gt 1 ]]; then
  # lipo rewrites the Mach-O, so it has to happen before codesign further down —
  # gluing slices onto a signed binary invalidates the signature.
  lipo -create -output "${APP}/Contents/MacOS/nudge" \
    $(printf 'target/%s/release/nudge ' "${TARGETS[@]}")
  echo "    slices: $(lipo -archs "${APP}/Contents/MacOS/nudge")"
else
  cp "target/${TARGET}/release/nudge" "${APP}/Contents/MacOS/nudge"
fi

cat > "${APP}/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key><string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleExecutable</key><string>nudge</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    ${LSUIELEMENT}
</dict>
</plist>
PLIST

plutil -lint "${APP}/Contents/Info.plist" >/dev/null

# Ad-hoc signature. arm64 code will not execute at all without at least this,
# so it is a build step rather than a release nicety. It is NOT a substitute
# for Developer ID: a bundle downloaded from GitHub Releases carries a
# quarantine attribute, and macOS 15 removed the Control-click override that
# used to let a user past that. On another Mac this bundle needs either
#     xattr -dr com.apple.quarantine /Applications/Nudge.app
# or a trip through System Settings › Privacy & Security › Open Anyway.
#
# TODO: when a Developer ID cert exists, sign with `--options runtime
# --timestamp -s "Developer ID Application: ..."`, then notarytool submit
# --wait and stapler staple. The bundle layout here is already what that
# pipeline expects; only the identity and the two extra flags change.
echo "==> Signing (ad-hoc)"
codesign --force --deep -s - "${APP}"
codesign --verify --verbose=2 "${APP}"

echo "==> Built ${APP}"
echo "    LSUIElement: $([[ "${AGENT_MODE:-1}" == "1" ]] && echo "on" || echo "withheld")"

# DMG=1 wraps the bundle for distribution. The staging directory carries an
# /Applications symlink so the disk image opens with the usual drag target.
if [[ "${DMG:-0}" == "1" ]]; then
  echo "==> Building DMG"
  DMG_PATH="${OUT_DIR}/${APP_NAME}-${VERSION}-macos.dmg"
  STAGE="$(mktemp -d)"
  cp -R "${APP}" "${STAGE}/"
  ln -s /Applications "${STAGE}/Applications"
  rm -f "${DMG_PATH}"
  hdiutil create -volname "${APP_NAME} ${VERSION}" -srcfolder "${STAGE}" \
    -ov -format UDZO "${DMG_PATH}" >/dev/null
  rm -rf "${STAGE}"
  echo "    ${DMG_PATH} ($(du -h "${DMG_PATH}" | cut -f1))"

  # Downloading this DMG anywhere else attaches com.apple.quarantine, and an
  # ad-hoc signature does not satisfy Gatekeeper. macOS 15 removed the
  # Control-click override, so the receiving user needs either
  #     xattr -dr com.apple.quarantine /Applications/Nudge.app
  # or System Settings › Privacy & Security › Open Anyway. Say so in the
  # release notes rather than letting people hit a bare "damaged" dialog.
fi

# TODO: AppIcon.icns into Contents/Resources + CFBundleIconFile. The tray daisy
# in src/daisy.rs is generated at runtime and is 64x64; the app icon is a
# separate asset that does not exist in the repo yet.
