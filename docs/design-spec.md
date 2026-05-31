# Nudge — style and behavior specification

Description at the level of feel and functionality, not pixel-perfect layout.
Implementation-independent — a person or model should be able to reproduce
the same usability and look from this document alone.

Nudge ships in two build surfaces:

- **Native Windows build** — the real app the user runs (tray icon, popup
  window, global hotkey, autostart, on-disk journal and config).
- **Browser build** — a stripped-down surface used for visual verification
  and end-to-end tests in a browser. It mirrors the popup card and a
  tray-emulating pill but has no real tray, no global hotkey, and no OS
  integrations; its journal and config live in browser storage.

Where the two surfaces differ in behavior (e.g. the tray exists only on
native, the pill exists only in the browser), the relevant section says so
explicitly. Otherwise the spec applies to both.

## 1. Window and background

- No system title bar or frame.
- Always on top of other windows while visible.
- Not resizable.
- Does not appear in the taskbar or in ALT+TAB.
- Lives in the tray: closing the popup does not exit the app.
- The window is transparent in both builds. The browser build draws a
  static decorative backdrop behind the card so the card's transparency and
  shadow are visible during in-browser verification; the native build has
  the desktop or other windows behind it.
- Palette: only shades of gray, no accent color.
- Card position: horizontally centered. Vertically — the top edge of the card
  sits at 25% of screen height from the top (the upper third, at natural eye
  level, noticeably above the geometric center). The card grows downward
  from this anchor.

## 2. Card

- Surface: semi-transparent with blur (frosted glass). Whatever is behind the
  window — desktop or other apps — shows through blurred.
- Width is around 480px; height grows with content.
- Corner radius is noticeable and soft.
- Shadow: minimal by baseline; slightly softer is acceptable for aesthetics.
- Three rows of equal height, separated by thin horizontal lines. The margin
  between the outer rows and the card edges matches the margin between fields.

## 3. Input fields

- Labels: placeholder inside the field, disappears on input. No separate
  labels above or beside.
- Placeholder texts (kept in Russian as the product copy of the questions):
  - Field 1: `Что я делаю?`
  - Field 2: `Хуйня?`
  - Field 3: `Следующий через (мин)`
- Focus is on the first field on open.
- Navigation: Tab forward, Shift+Tab backward.
- The active (focused) row is highlighted by a subtle background change. No
  borders, outlines, or color accents.
- The minutes field accepts integers and decimals (e.g. `0.1` = 6 seconds, for
  testing).
- Field state on close is specified by the table in §4. In short: only Enter
  clears `doing` / `bullshit`. Esc and Switch preserve them — on the next
  open the popup continues from where it left off.
- The minutes field always persists the last entered text after any close
  (Enter, Esc, Switch) — on the next open the popup shows that value. It does
  not change without explicit editing.

## 4. Behavior

The popup is closed by three actions. Esc and Switch behave the same — only
the trigger differs (key press vs. focus loss). The distinction would
otherwise be a hidden rule, so they are merged into one row.

| Action | What it does |
|---|---|
| **Enter** | If at least one of `doing` / `bullshit` is non-empty — writes a journal entry. Restarts the timer using the minutes field. Clears `doing` / `bullshit`. |
| **Esc** / **Switch** (click outside the card / window focus loss) | Preserves `doing` / `bullshit` — on the next open the popup continues from where it was. Does not write to the journal. Does not touch the timer (exception below). |

All three hide the popup.

**When Esc or Switch do restart the timer.** If the popup was opened *by the
timer* (not manually), the timer is already at zero. Closing the popup in
that state without setting a new deadline would make it appear again
immediately — so in this case Esc and Switch must restart the timer. This is
a structural constraint, not a semantic choice. Whenever the timer is
updated, it always uses the current value of the minutes field.

**The minutes field** always preserves the last entered text on any close
(see §3) — even Esc and Switch do not revert it to a previous value.

**Switch by window focus loss** only fires if the popup actually received
focus when it was opened. If it didn't (e.g. a fullscreen app held
foreground), focus loss does not close it; Enter, Esc, and clicks outside
the card still do.

**Open the popup manually** — click on the tray icon or pill, even if the
timer has not expired yet. Opening this way does not affect the timer.

**"Change the interval without writing to the journal" scenario:** open the
popup manually → adjust the minutes field → Enter with empty `doing` and
`bullshit`. Nothing is written; the timer is rearmed.

**First launch** shows the popup right away (as if the timer had expired):
the initial `trigger_source` is `timer`, and the first close (by any means)
starts the timer. After that the cycle is hide → timer → popup → ...

Popup show and hide are instant, no animations.

## 5. Tray (native Windows build only)

The tray icon is a live indicator of time-to-next-nudge: it both shows the
app is running and "ripens" instead of a numeric countdown.

### Icon as an object

- Daisy (Bellis perennis): 12 plump elliptical petals around a bright yellow
  center. The white petal base softens into a warm pink at the very tips —
  the daisy's signature, distinguishing it from a generic field chamomile.
- Just the daisy and its center. No frame, background, shadow, digits, or
  any text inside the icon.
- The palette is the only place in the app with an accent color (pink tip +
  yellow center). The card and pill stay monochrome.

### Link to the timer

- The flower "counts down" the current interval, dropping petal by petal.
- At the moment the timer is rearmed, all 12 petals are present.
- Over the interval exactly 12 petals fall, evenly spaced (one per
  `interval / 12`).
- By the time the timer expires only the center remains on the stem.
- Order: the top petal (12 o'clock) falls first, then clockwise. The order
  is deterministic, not random — it is part of the character's "life" and
  the user picks up on it over time.
- Behavior depends **only** on the current interval and elapsed time. The
  contents of `doing` / `bullshit` do not affect the icon.

### Fall animation

- A petal does not vanish instantly. After it detaches it is visible for
  ~250ms: it drifts outward from the center, sags slightly downward (as if
  under light gravity), and simultaneously fades to transparent.
- On very short intervals the fall duration is shortened so one falling
  petal does not collide with the next. The animation always finishes
  before the next petal detaches.
- The popup appears not at the moment the last petal detaches, but after
  its fall has played out completely. The user sees the flower fully
  "stripped" before the card appears.

### Interaction

- Tooltip: `~N min`, rounded up to a whole minute, updated once per minute
  (no seconds). After the timer expires: `now`.
- Left click on the icon: show the popup (the browser build's equivalent
  gesture is a click on the pill, see §6). Does not affect the timer — it
  keeps running.
- Right click: context menu with three items: `Show Nudge`, `Settings`,
  `Quit`. `Settings` opens the settings window (§9).
- Tray language (tooltip and menu) is English. The popup card stays in
  Russian for the question placeholders (§3); the settings window is in
  English.

### States outside the normal cycle

- While the popup is open the icon stays in whatever state it was at the
  moment of opening (the animation freezes). It is redrawn from scratch
  when the popup closes and the timer starts again.
- The icon does not react to pause, journal errors, or other out-of-band
  events — it strictly reflects "time left to the next nudge".

### Global hotkey

- A system-wide hotkey opens the popup from any focused window. The effect
  is identical to a left click on the tray icon: the popup opens with
  `trigger_source = manual`, and opening does not affect the timer.
- The combination is configured in `<Documents>/Nudge/config.json`, field
  `hotkey`. Default: `Ctrl+Shift+Space`. Format: modifiers (`Ctrl` /
  `Alt` / `Shift` / `Win`) and one key (letter, digit, `F1`..`F24`,
  `Space`, `Enter`, `Tab`, `Escape`, `Backspace`) joined by `+`. Case and
  whitespace around `+` are ignored.
- If the config file is absent — a default is written on first launch. An
  invalid string or a combo already taken by another app is not fatal: the
  app starts without a hotkey and the error goes to stderr. The popup is
  still reachable via the tray icon.

## 6. Pill (browser build only — tray emulation)

- Rendered only in the browser build, only while the popup is hidden.
- Position: bottom-right corner, ~16px from the edges.
- Shape: elongated pill, fully rounded ends.
- Background: flat dark gray, darker than the card. No frosted glass.
- Contents: time only, format `M:SS` (minutes:seconds), no prefix. Updated
  once per second.
- Hover: background slightly lighter.
- Click: open the popup.

## 7. Journal

Not visually exposed, but it defines the error path:

- Append-only NDJSON (one JSON record per line). Full contract in
  `docs/journal-spec.md`.
- On a journal write error (bad format, no permissions) — show a system
  dialog, keep the popup open, do not lose the typed text.

## 8. Open behavioral tasks

Not visual decisions, but they follow from the spec:

- Frosted-glass backdrop behind the card on the native Windows build. The
  card sits on top of a transparent window today, but the surrounding blur
  the §2 frosted-glass rule asks for is not yet wired up — it is an open
  task. The browser-build backdrop is described in §1.

## 9. Settings

The settings window is a separate surface from the popup, with its own chrome
(title bar, frame, close button). It is an alternative way to edit the same
file that lives at `<Documents>/Nudge/config.json` (§5) — no new entity, just
a GUI on top of the existing config.

### Opening

- Right click on the tray icon → `Settings` menu item (§5: tray language is
  English).
- The settings UI is a **separate surface from the main app**. The popup,
  timer, tray icon, and global hotkey keep working while it is open; the
  two surfaces communicate exclusively through `config.json` (and the
  Windows registry, for autostart) — no in-process channel.
- If the settings window is already open, another click does nothing — at
  most one settings instance exists at a time.

### Fields

| Field | Type | What it does |
|---|---|---|
| Global hotkey | text + recorder | A string in the §5 format (e.g. `Ctrl+Shift+Space`). Next to it is a `Record` button: clicking it puts the field into capture mode. The first supported combo (letters A-Z, digits 0-9, F1-F24, Space/Enter/Tab/Escape/Backspace + modifiers) is canonicalized via the §5 parser and dropped into the field. Bare Escape cancels recording and restores the previous value; an unsupported key keeps the field in recording mode with a hint. Recording only edits the staged value — `Save` is still required to write to `config.json`. |
| Default interval (min) | text | A number, must be finite and positive; same rules as the minutes field of the popup in §3. |
| Launch with Windows | checkbox | Controls registering the app to launch at session start (§5 — Windows only). |

### Saving

- **Hotkey and interval** are written to `config.json` by the `Save` button.
  If the interval does not parse — a banner shows the error and the file is
  not touched.
- **Autostart** is applied **immediately** on a checkbox click — under the
  transactional rule: first change OS state (registry), then confirm the
  system reports the new state, and only then write `autostart: true/false`
  to `config.json`. If the OS refused or did not confirm — the checkbox
  stays in its previous state and the user sees an error in the banner.
  `config.json` never claims a state that is not present in the registry.

### Applying changes

Edits saved from the settings window (or made by hand in `config.json`)
are picked up by the running app **immediately**: the main app watches
the file and re-reads it on every change. Saves are atomic — the watcher
must never observe a half-written file. Malformed JSON is logged to stderr
and the app keeps running with the previous config.

Of those changes, only the **hotkey** has a live effect on a running
popup: the main app drops the old system-wide registration and binds the
new combination right away. The other fields are re-read and cached for
consistency but have no active UI effect:

- `default_interval_minutes` — the popup picks it up at startup and then
  owns its own "next nudge in" field; live-reload does not overwrite
  whatever the user is currently typing.
- `autostart` — managed transactionally from the settings window (see
  above); the main app does not touch the registry and does not try to
  reconcile it against `config.json` — that is an adjacent concern.

### Window chrome

- Standard OS title bar and frame (unlike the popup — this is not a
  spotlight).
- Title: `Nudge — Settings`.
- `Save` / `Cancel` buttons. Esc and the system close button are equivalent
  to Cancel: the window closes without saving the fields that are saved
  explicitly (hotkey, interval). An autostart change already applied is of
  course not rolled back — it was committed at the moment of the click.
