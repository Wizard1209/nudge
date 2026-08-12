# Nudge

Interval journaling for focus. Native Rust, Windows.

@README.md

## Dev Commands

### Build & run native (Windows exe via WSL)
```sh
cargo build --bin nudge --target x86_64-pc-windows-gnu
./target/x86_64-pc-windows-gnu/debug/nudge.exe
```

### Build & run native (macOS)
```sh
cargo build --bin nudge && ./target/debug/nudge

# The shipped form is a bundle, and it is not optional packaging: LSUIElement
# and Launch Services agree with the running app only from inside one.
DMG=1 ./scripts/build-macos-app.sh    # -> target/macos/Nudge.app + .dmg
```

### Run the macOS native tests
```sh
# Drives tests/macos_native.rs — the port's acceptance suite, and that suite
# alone — against a real desktop inside a Tart guest. The hotkey tier below is
# a separate run on purpose: it re-records tests/fixtures/keystreams, which is
# not something a plain test run should do behind your back.
# Do this even with your own screen unlocked: behind a lock screen synthetic
# input, activation and rect captures all fail, and the appearance tests need
# a desktop with no full-screen window on it (a full-screen app sits in its own
# Space, where the control backdrop cannot follow). The guest is the
# reproducible path rather than the fallback.
scripts/native-tests-macos.sh
scripts/native-tests-macos.sh -- popup_reappears    # one test

# The hotkey recorder's tier. It presses real keys AND records the streams
# tests/hotkey_replay.rs replays on the host, so a run here is what gives the
# fast tier its inputs — see tests/fixtures/keystreams/README.md.
SUITE=macos_hotkey_native scripts/native-tests-macos.sh

# The vision-judge test is a no-op pass without OPENAI_API_KEY in the guest's
# environment:
GUEST_ENV="OPENAI_API_KEY=$OPENAI_API_KEY" scripts/native-tests-macos.sh -- vision_judge
```

### Build WASM (for e2e tests only)
```sh
# Builds both popup (index.html) and settings (settings.html) into e2e/dist.
# A single `trunk build e2e/index.html` is NOT enough — it leaves settings.html
# out of dist and e2e/tests/settings.test.ts times out on the missing page.
scripts/build-e2e.sh
```

### Run e2e tests
```sh
# Terminal 1: serve the dist built by scripts/build-e2e.sh
python3 -m http.server 8080 --directory e2e/dist

# Terminal 2: run tests (from e2e/)
# Standard deterministic suite (what per-push CI runs):
cd e2e && nvm use && npx vitest run
# LLM-judge group (*.judge.test.ts, needs OPENAI_API_KEY in ../.env;
# CI runs it only in the release workflow):
npm run test:judge
```