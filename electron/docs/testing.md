# Testing workflow & conventions

Three tiers, two roles. **Maximize what the fast tiers cover; reach for native
only for final validation.**

## The convention

- **During development — unit + browser e2e drive everything.** Do red-green TDD
  and catch regressions with `pnpm test:unit` and `pnpm test:e2e`. They're fast,
  deterministic, and run in CI/WSL. Anything that *can* be tested here *should*
  be: pull logic into pure modules (`src/shared/*`) and the browser Settings
  sub-app (`/settings.html`) so it lands in these tiers rather than native.
- **On task completion — native tests give the final validation.** Run
  `pnpm test:native` once the feature is done, as the gate before shipping a
  build. Don't use it for the moment-to-moment red-green loop — it's slow, needs
  a display, must run unsandboxed, and some of its invariants only fail on
  Windows (so a green native run on Linux is a smoke check, not proof — see the
  platform note in `manual-verification.md`).

Rule of thumb: if you're tempted to write a native test to drive a red-green
cycle, first ask whether the behavior can be extracted into a pure module or the
browser sub-app and tested there instead. Native is the last line, not the first.

## The tiers

| Tier | Command | Speed | Env | Covers |
|------|---------|-------|-----|--------|
| Unit | `pnpm test:unit` | fast | node, sandbox OK | pure logic: hotkey parse/format/recorder, config load/save/diff, SettingsForm, autostart transaction |
| Browser e2e | `pnpm test:e2e` | medium | needs `pnpm dev`, **unsandboxed** | renderer behavior via Puppeteer: popup interactions, the Settings sub-app at `/settings.html` |
| Native | `pnpm test:native` | slow | built app + display (WSLg), **unsandboxed** | real Electron main process + native windows: focus trap, window geometry/background, tray/quit lifecycle, recording a hotkey in the real Settings window |

Notes:
- Browser e2e and native both need the process **unsandboxed** (Vite's localhost
  binding / Electron's X socket aren't reachable from the sandbox).
- Browser e2e needs Puppeteer's browser in `.puppeteer-cache/`. `pnpm install`
  downloads it, but only the first time on a machine: pnpm restores puppeteer
  from its side-effects cache afterwards and skips the postinstall, which is
  where the download lives. `pnpm test:e2e` checks for it and names the command
  that puts it back.
- **Focus loss is not observable in either harness.** Puppeteer's
  `bringToFront` does not blur the previous tab, and an Electron window driven
  by Playwright never becomes the OS's focused window — `isFocused()` stays
  false, so `blur()` has nothing to give up. A behavior gated on focus is
  tested by moving focus into a subframe (browser tier), or by driving the
  event the OS would have produced and asserting everything downstream of it
  (native tier). Say which one a test does.
- Native loads the **built** renderer (`NUDGE_FORCE_PROD`), so run
  `pnpm build && pnpm build:electron` first — it validates the exact assets the
  portable `.exe` ships.

## Ship checklist

1. `pnpm typecheck`
2. `pnpm test:unit` && `pnpm test:e2e` (green throughout development)
3. `pnpm build && pnpm build:electron`
4. `pnpm test:native` (final native gate)
5. Native-only manual items (global hotkey, registry autostart, config watcher,
   Windows acrylic rendering) — `manual-verification.md`.
