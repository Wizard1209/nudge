# Nudge — interval journaling for focus

A periodic spotlight popup that asks you "what are you doing?" and "is it
bullshit?". Answers are written to an append-only NDJSON journal.

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
- **Esc** — close without saving (the timer still restarts)

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
- Journal: `%USERPROFILE%\Documents\Nudge\journal-rust.ndjson`
- Hotkey: **Ctrl+Shift+Space** — opens the popup manually from any focused
  window. Field `hotkey` in `%USERPROFILE%\Documents\Nudge\config.json`.
  Format: modifiers (`Ctrl`, `Alt`, `Shift`, `Win`) + one key joined by `+`,
  e.g. `Alt+J` or `Ctrl+F12`.
- Launch with Windows: field `autostart` (bool) — kept in sync with
  `HKCU\…\Run\Nudge`; see `docs/design-spec.md` §9.

## Settings

The settings window opens from the tray menu (`Settings`) and edits the same
`config.json` (hotkey, interval, autostart). Edits are picked up by the
running app immediately via a file watcher. Full contract:
`docs/design-spec.md` §9.

## TODO (post-MVP)

- [ ] Audio cue on popup.
- [ ] LLM classifier: auto-evaluate "bullshit or not" from the text of both
      fields (if not stated explicitly → `null`). Lightweight model, local or
      API.
- [ ] Voice input via ElevenLabs STT — a button / hotkey in the popup for
      dictation instead of typing.
