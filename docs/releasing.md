# Releasing

Releases are cut by pushing a tag `vX.Y.Z` from a green `master`. The tag
triggers `.github/workflows/release.yml`: the full e2e surface (deterministic
suite plus the LLM-judge group, which needs the `OPENAI_API_KEY` repo secret)
gates the build, then the cross-built `nudge.exe` is attached to a GitHub
Release.

## Before tagging

1. **Green master** — working tree clean, everything pushed, CI green on the
   commit you are about to tag.
2. **Version** — `version` in `Cargo.toml` matches the tag (`v0.1.0` ↔
   `0.1.0`).
3. **Windows-only ignored tests** — `scripts/native-tests.sh` runs
   `native_render` (real window render + LLM vision judge); `perf_idle` (idle
   CPU budget) is not in it and runs on its own:
   `cargo test --target x86_64-pc-windows-gnu --test perf_idle -- --ignored --nocapture`.
   These are local-only for now — CI has no Windows runner with a live
   desktop. Skip only if they already ran on the same code recently.
4. **macOS ignored tests** — run `scripts/native-tests-macos.sh`, which drives
   `tests/macos_native.rs` inside a Tart guest. Every test in it must pass.
   They stay out of CI for the same reason as the Windows pair, plus one of
   their own: Screen Recording and Accessibility are baked into a runner
   image ahead of time and cannot be granted during a job.

   Run them in the guest even when your own screen is unlocked. Behind a lock
   screen they cannot run at all — synthetic input, application activation and
   rect captures all fail — so the guest is the reproducible path rather than
   the fallback.

   Then `scripts/perf-macos.sh` — the macOS counterpart of `perf_idle`:
   `tests/perf_macos.rs` holds the release build to its hidden-footprint,
   re-show growth, idle-CPU and show-latency budgets and prints the numbers as
   a `PERF_MACOS_JSON` line. It runs on your own unlocked desktop rather than
   in the guest, because the guest script ships debug builds and measures on a
   different display; it needs no grant, but it takes the keyboard for about
   75 s.
5. **Manual smoke of a release build on live Windows** — popup opens focused;
   Enter appends a journal line; Esc closes without writing; tray daisy and
   menu work; global hotkey opens the popup; settings window saves.
6. **Manual smoke of the macOS bundle** — `DMG=1 ./scripts/build-macos-app.sh`,
   install `Nudge.app`, and check the same list. Two extra items that only
   exist here: no Dock icon (`lsappinfo` should report `UIElement`), and Quit
   from the menu-bar daisy actually exits — with `LSUIElement` on there is no
   other way out.

## Tag

7. `git tag vX.Y.Z && git push origin vX.Y.Z`
8. Watch the Release workflow to completion — the judge e2e group failing
   loudly (missing/expired `OPENAI_API_KEY` secret) blocks the release, which
   is intended. The macOS job builds a universal `.app`, wraps it in a DMG and
   hands it to the release job as an artifact, so a single job publishes both
   files and two runners cannot race to create the tag.

## After

9. Release page: the `.exe` and the `.dmg` are both attached, generated notes
   look sane, and the notes carry the quarantine instructions.
10. Download the `.exe` from the Release page — not a local build — and smoke
    it on Windows.
11. Download the `.dmg` the same way and install it on a Mac that has never
    built the app. This is the only step that exercises Gatekeeper for real:
    a locally built bundle carries no quarantine attribute, so it cannot show
    you what a downloading user sees. Expect the first launch to be refused
    and the documented `xattr -dr com.apple.quarantine` to clear it.
