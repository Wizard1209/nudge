# Nudge

Interval journaling for focus. Native Rust, Windows.

@README.md

## Dev Commands

### Build & run native (Windows exe via WSL)
```sh
cargo build --bin nudge --target x86_64-pc-windows-gnu
./target/x86_64-pc-windows-gnu/debug/nudge.exe
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
cd e2e && nvm use && npx vitest run
```