# Manual verification checklist — native-only features

Most of the v2 spec (config parsing, hotkey parse/format/record, Settings form
logic, autostart transaction) is covered by automated tests:

- **Unit** (`pnpm test:unit`) — hotkey parse/format/accelerator/recorder,
  config load/save/diff, SettingsForm, autostart transaction.
- **Browser e2e** (`pnpm test:e2e`, needs `pnpm dev`) — the Settings sub-app at
  `/settings.html`: recorder captures a chord, interval/autostart/Save persist,
  bare-Escape cancels recording.
- **Native** (`pnpm test:native`) — launches the **real** Electron app
  (Playwright `_electron`) against the *built* renderer (`pnpm build &&
  pnpm build:electron` first). Covers the native-window regressions browser
  tests can't see: the popup's Tab focus-trap (never escapes the frameless
  window), the Settings window's dark surface + fit (no gray flash, no
  clipping), and a clean tray teardown on quit (no "Object has been destroyed").
  **Needs a display (WSLg/X11) and must run UNSANDBOXED** so Electron can open
  the X socket.

  Platform note: a few of these invariants only *fail* on Windows (frameless
  focus leak, title-bar inset, tray-destroy race). On Linux/WSLg Chromium
  default-wraps focus and the compositor doesn't inset the frame, so those
  assertions pass regardless — the suite is the regression guard that goes red
  on a real Windows build. The dark-surface and quit-crash assertions are
  reproducible on Linux too.

The items below depend on a real OS session (global hotkey delivery, the
Windows registry, the filesystem watcher) and can't be automated in the
headless CI/WSL environment. Verify them by hand on a Windows build
(`pnpm dist`, then run the packaged app).

## Global hotkey (§5)

- [ ] With the app running, the configured hotkey (default `Ctrl+Shift+Space`)
      opens the popup from **another focused application** (e.g. a maximized
      browser or editor).
- [ ] An **invalid** hotkey in `config.json` (e.g. `"hotkey": "Ctrl+Nope"`) →
      app still launches, tray works, stderr logs the ignored hotkey, no crash.
- [ ] A hotkey **already taken** by another app → stderr logs "unavailable",
      app continues without that hotkey.

## Config file watcher → live re-register (§9)

- [ ] Hand-edit `config.json` (`<Documents>/Nudge/config.json`), change only the
      `hotkey`, save → the **old** combo stops working and the **new** one opens
      the popup, without restarting the app (≤ ~1s).
- [ ] Edit only `default_interval_minutes` or `autostart` → no live effect
      (interval applies to the next nudge, autostart to the next launch); no
      errors logged.
- [ ] Write malformed JSON → previous config stays in effect, error logged, no
      crash.

## Settings window (§9)

- [ ] Tray → **Settings** opens a single framed window titled
      "Nudge — Settings"; opening it again while open **focuses the existing
      one** (no second window).
- [ ] **Record** captures a chord pressed inside the window; the label updates.
- [ ] **Save** writes `config.json` and the new hotkey takes effect immediately.
- [ ] **Cancel** (and the window's close button) discards edits and closes.
- [ ] Invalid interval on Save → banner error, nothing persisted.

## Autostart (§9)

- [ ] Toggling **Launch at login** flips the Windows registry `Run` entry
      immediately (no Save needed) — confirm via `regedit`
      (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`) or by rebooting.
- [ ] If the registry write is refused, the checkbox **reverts** and a banner
      shows the failure (config is not left claiming autostart is on).

## Focus-loss Switch refinement (§4)

- [ ] Trigger the popup over a **fullscreen** app that keeps the foreground:
      the popup does **not** instantly Switch-away from a stray blur it never
      had focus for.
- [ ] Normal case (popup gets focus) still Switches on blur as before.
- [ ] **Known limitation / to confirm:** a focus→immediate-blur *bounce* right
      after show isn't specifically suppressed (the renderer treats any real
      focus as "got focus"). Watch for the popup vanishing instantly in this
      case; tighten the gate (e.g. a short post-show grace window) if observed.
