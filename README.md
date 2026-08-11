# Nudge — interval journaling for focus

A periodic spotlight popup for Windows and macOS that asks you "what are you
doing?" and "is it bullshit?". Answers are written to an append-only NDJSON
journal.

![Nudge popup](assets/popup.png)

## How it works

1. Nudge sits in the system tray, the timer ticks.
2. Timer expires → a spotlight popup appears in the center of the screen
   (always on top, like Spotlight / Claude quick chat).
3. The user answers → Enter → the popup hides, the timer restarts.
4. Answers are appended to `journal-rust.ndjson`.

## Popup

Minimalist window in the center of the screen, on top of all other windows.
Opens focused on the first field.

### Fields (Tab to switch)

| # | Field (Russian product copy — the actual questions) | Type | Description |
|---|------|-----|----------|
| 1 | Что я делаю? | text | Free-form input, focused on open |
| 2 | Не хуйню ли я делаю? | text | Free-form input, reflection |
| 3 | Следующий nudge через | number (min) | Pre-filled with the current interval (default 10 min) |

- **Enter** — save and close
- **Esc** — close without saving (whether the timer restarts depends on how
  the popup was opened — see below)

Everything the popup does is one state machine: three states, four events,
no rules outside the transition table. It is specified in
[docs/design-spec.md](docs/design-spec.md) §4 and there is a clickable model
of it in [docs/state-machine.html](docs/state-machine.html).

## Journal

NDJSON file (`journal-rust.ndjson`), append-only, one JSON record per line:

```jsonl
{"schema_version":1,"event_type":"submitted","entry_id":"01JS1S8R5W4Y4S4M8Q6A8X7R2V","captured_at":"2026-04-08T14:30:00.000+03:00","implementation":"rust","trigger_source":"timer","doing":"writing requirements","bullshit":"no","next_interval_minutes":10}
{"schema_version":1,"event_type":"submitted","entry_id":"01JS1S9FDRW4K4M7R4F5R9A5A2","captured_at":"2026-04-08T14:40:00.000+03:00","implementation":"rust","trigger_source":"timer","doing":"got sucked into YouTube","bullshit":"yes","next_interval_minutes":5}
```

Full contract: [docs/journal-spec.md](docs/journal-spec.md)

## Tray

- Tray icon — a daisy that drops petals as the next nudge approaches (this
  is the time-remaining indicator, see `docs/design-spec.md` §5).
- Tooltip: `~N min` (rounded up, refreshed once per minute); `now` after the
  timer expires.
- Left click: open the popup (does not affect the timer).
- Right click: context menu — `Show Nudge`, `Settings`, `Quit`.

## Stack

- **Rust** (native, no web engines).
- GUI: eframe / egui (glow renderer, transparent window).
- Goal: minimal footprint (~10-20 MB RAM), instant startup, single binary.

## Defaults

- Interval: **10 minutes** — field `default_interval_minutes` in
  `config.json`, any positive number (integer or decimal).
- Journal: `<Documents>/Nudge/journal-rust.ndjson` — `%USERPROFILE%\Documents`
  on Windows, `~/Documents` on macOS.
- Config: `%USERPROFILE%\Documents\Nudge\config.json` on Windows,
  `~/Library/Application Support/Nudge/config.json` on macOS. Not next to
  the journal there: Documents is TCC-gated and iCloud-synced, and a
  menu-bar agent should not have to ask for consent to read its own
  settings. A file left in `~/Documents/Nudge/` by an earlier version is
  moved on first launch.
- Hotkey: **Ctrl+Shift+Space** (`⌃⇧Space`) — opens the popup manually from
  any focused window. Field `hotkey` in `config.json`: any modifiers plus
  one key. `Ctrl`/`⌃`, `Alt`/`Option`/`⌥`, `Shift`/`⇧`,
  `Cmd`/`Win`/`⌘`, plus a letter, digit, `F1`–`F24`, `Space`, `Enter`,
  `Tab`, `Esc` or `Backspace` — e.g. `⌥⌘J`, `Alt+J`, `Ctrl+F12`. Case,
  spacing and the choice of spelling do not matter. `fn` cannot be part of
  a hotkey on macOS; see `docs/design-spec.md` §5 for why.
- Launch at login: field `autostart` (bool) — kept in sync with
  `HKCU\…\Run\Nudge` on Windows and with a LaunchAgent plist at
  `~/Library/LaunchAgents/com.wizard1209.nudge.plist` on macOS; see
  `docs/design-spec.md` §9.

## Installing on macOS

Download the DMG from the Releases page, drag `Nudge.app` to Applications,
then clear the download quarantine:

```sh
xattr -dr com.apple.quarantine /Applications/Nudge.app
```

That step is needed because the app carries an ad-hoc signature rather than a
Developer ID, and macOS 15 removed the Control-click override that used to get
past Gatekeeper. The alternative is one trip through System Settings › Privacy
& Security › Open Anyway after the first launch is refused.

Nudge runs as a menu-bar agent: no Dock icon, no Cmd-Tab entry, and the daisy
in the menu bar is where Settings and Quit live.

## Settings

The settings window opens from the tray menu (`Settings`) and edits the same
`config.json` (hotkey, interval, autostart). Edits are picked up by the
running app immediately via a file watcher.

The hotkey is a text field — type any of the spellings above; there is no
capture button, and `docs/design-spec.md` §9 says why. `Save` refuses a
combination the OS cannot bind rather than writing it. Full grammar:
`docs/design-spec.md` §5.

## Development: the Electron prototype

This repo holds two implementations of the same spec. Alongside the Rust app
lives [`electron/`](electron/README.md) — the **prototyping harness** where new
features and design changes are tried first, because Electron is fast to build
and easy to inspect visually. The outcome is captured in the shared spec under
[`docs/`](docs), the contract both implementations follow, and then ported to
Rust. The released product is the Rust binary only; the Electron app is a
workbench, fully rewritable from the spec.
