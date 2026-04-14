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
trunk build e2e/index.html --dist e2e/dist
```

### Run e2e tests
```sh
# Terminal 1: serve WASM build (from e2e/)
cd e2e && trunk serve

# Terminal 2: run tests (from e2e/)
cd e2e && nvm use && npx vitest run
```