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
3. **Windows-only ignored tests** — run `scripts/native-tests.sh`:
   `perf_idle` (idle CPU budget) and `native_render` (real window render +
   LLM vision judge). These are local-only for now — CI has no Windows runner
   with a live desktop. Skip only if they already ran on the same code
   recently.
4. **Manual smoke of a release build on live Windows** — popup opens focused;
   Enter appends a journal line; Esc closes without writing; tray daisy and
   menu work; global hotkey opens the popup; settings window saves.

## Tag

5. `git tag vX.Y.Z && git push origin vX.Y.Z`
6. Watch the Release workflow to completion — the judge e2e group failing
   loudly (missing/expired `OPENAI_API_KEY` secret) blocks the release, which
   is intended.

## After

7. Release page: the `.exe` is attached, generated notes look sane.
8. Download the `.exe` from the Release page — not a local build — and smoke
   it on Windows.
