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
#   * `LSUIElement` (no Dock icon, no Cmd-Tab entry) is a bundle property, and
#     it is what replaces `with_taskbar(false)` + `WS_EX_TOOLWINDOW`.
#
# Usage:
#   scripts/build-macos-app.sh                 # debug-friendly: Dock icon kept
#   AGENT_MODE=1 scripts/build-macos-app.sh    # LSUIElement=1, menu-bar only
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

# LSUIElement hides the app from the Dock and Cmd-Tab. Do NOT default it on:
# the macOS build currently has no menu-bar status item (tray-icon is confined
# to the Windows dependency table), so an agent-mode bundle would have no icon,
# no menu, and no visible window to close — it could only be killed from the
# command line. Flip this on in the same change that lands the status item.
if [[ "${AGENT_MODE:-0}" == "1" ]]; then
  LSUIELEMENT="<key>LSUIElement</key><true/>"
else
  LSUIELEMENT="<!-- LSUIElement withheld: no status item exists yet to quit from -->"
fi

echo "==> Building ${APP_NAME} ${VERSION} for ${TARGET}"
cargo build --release --bin nudge --target "${TARGET}"

echo "==> Assembling bundle"
rm -rf "${APP}"
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"
cp "target/${TARGET}/release/nudge" "${APP}/Contents/MacOS/nudge"

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
echo "    LSUIElement: $([[ "${AGENT_MODE:-0}" == "1" ]] && echo "on (no Dock icon)" || echo "off (Dock icon kept)")"

# TODO: universal binary — add x86_64-apple-darwin, lipo the two slices before
# signing. Ordering matters: lipo rewrites the Mach-O, so signing first would
# invalidate the signature.
# TODO: AppIcon.icns into Contents/Resources + CFBundleIconFile. The tray daisy
# in src/daisy.rs is generated at runtime and is 64x64; the app icon is a
# separate asset that does not exist in the repo yet.
# TODO: DMG via hdiutil create -volname -srcfolder -format UDZO.
