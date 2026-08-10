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
  window — desktop or other apps — shows through blurred. The blur has to come
  from the OS compositor, since the renderer cannot sample pixels behind its
  own window: macOS supplies it, and there the fill is correspondingly lighter
  because its job is only to tint the frost. **Windows and the browser build
  have no blur today** and carry a near-opaque fill instead — without one, a
  translucent card leaves the desktop legible straight through it.
- Width is around 480px; height grows with content.
- Corner radius is noticeable and soft.
- Shadow: minimal by baseline; slightly softer is acceptable for aesthetics.
  Exactly one layer draws it. Where the compositor can — macOS, from the
  window's alpha mask — it is the compositor's, so the card casts the same
  shadow as any other floating panel on that system and the renderer draws
  none; elsewhere the renderer draws it, because nothing else will. Both at
  once is not "a stronger shadow": the compositor traces the alpha silhouette,
  which with a renderer shadow in it is larger than the card, so its edge falls
  in open space as a dark ring detached from the contour.
- Contour: a light hairline runs the full perimeter. It is what separates the
  card from the desktop when the shadow cannot — the shadow is dark, so over a
  dark window behind the popup it contributes nothing and the card would lose
  its boundary. Anything drawn inside the card stays clear of the hairline and
  concentric with it: a second curve at a different radius reads as a
  misaligned double outline at the corners.
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
- The combination is configured in `config.json` (§9), field `hotkey`. It is
  a **string**, and this section defines that string exactly. Default:
  `Ctrl+Shift+Space`.

#### Grammar

```
hotkey     = { modifier , [ "+" ] } , key ;

modifier   = control | option | shift | command ;
control    = "Ctrl"    | "Control"                              | "⌃" ;
option     = "Alt"     | "Option"  | "Opt"                      | "⌥" ;
shift      = "Shift"                                            | "⇧" ;
command    = "Cmd"     | "Command" | "Win" | "Super" | "Meta"    | "⌘" ;

key        = letter | digit | function | named ;
letter     = "A" … "Z" ;
digit      = "0" … "9" ;
function   = "F1" … "F24" ;
named      = "Space"                    | "␣"
           | "Enter"  | "Return"        | "↩" | "⏎" | "⌤"
           | "Tab"                      | "⇥"
           | "Esc"    | "Escape"        | "⎋"
           | "Backspace"                | "⌫" ;
```

Reading rules, all of them:

1. **Case is ignored.** `ctrl+a` and `CTRL+A` are the same string.
2. **Whitespace is ignored**, around the whole string and around every `+`.
3. **`+` is required between words, optional after a symbol.** `⌥⌘J`,
   `⌥+⌘+J` and `Alt+Cmd+J` are one hotkey. The symbol form needs no
   separator because that is how macOS writes a shortcut, and it is what the
   settings field holds there.
4. **Exactly one `key` is required.** Zero (`Ctrl+Shift`) and two
   (`Ctrl+A+B`) are both errors, with distinct messages.
5. **Repeated modifiers collapse.** `Ctrl+Ctrl+A` is `Ctrl+A`.
6. **Anything else is an error** naming the offending token.

#### Writing it back

One combination has one canonical spelling **per platform**, and that is
what the app writes and displays: `⌃⇧Space` on macOS, `Ctrl+Shift+Space` on
Windows. Modifiers are always emitted in the order **Control, Option/Alt,
Shift, Command/Win** — the order Apple prints them in a menu.

This is a rendering, not a second format. Every spelling in the grammar
parses on both platforms, so a `config.json` carried between a Mac and a PC
keeps its meaning; only the way it is written back changes.

#### What the OS can and cannot bind

- The fourth modifier is one modifier: `Cmd`, `Win` and `⌘` all name it. It
  registers as **Command** on macOS and as the **Windows key** on Windows.
- **`fn` is not in the grammar and cannot be.** Carbon's
  `RegisterEventHotKey` — the macOS registration call — takes four modifier
  bits (Shift, Command, Option, Control), and the window toolkit reports the
  same four, so `fn` reaches neither the binding nor the app. macOS also
  sets the `fn` flag by itself for arrows and the F-row, so it is not a
  modifier a user chooses in the first place.
- **`F1`–`F12` need `fn` held on a Mac** unless *Use F1, F2, etc. as
  standard function keys* is on. **`F21`–`F24`** satisfy the grammar but
  fail to register there.

#### When it cannot be applied

If the config file is absent, a default is written on first launch. A
combination that cannot be registered — one that breaks the grammar, or one
another app already holds — is not fatal, and **does not fall back to the
default**: the app runs **without a hotkey**, says so on stderr, and stays
reachable through the tray icon. Silently binding something else would take
over a shortcut the user never asked for while hiding which one failed.
Startup and live reload (§9) follow the same rule.

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
`config.json` — no new entity, just a GUI on top of the existing config.

### Where the file is

| | Path |
|---|---|
| macOS | `~/Library/Application Support/Nudge/config.json` |
| Windows | `%USERPROFILE%\Documents\Nudge\config.json` |

Neither depends on where the binary was launched from, so a build tree, a
copy in `/Applications` and a mounted DMG all read and write the same file.

macOS does not put it next to the journal, and both reasons are about
Documents rather than about Application Support: that folder is TCC-gated, so
the first read prompts for consent from a menu-bar agent with no window on
screen, and it is iCloud-synced on most machines, which would have two Macs
fighting over one hotkey and the file watcher (below) seeing edits nobody
made. The journal stays there on purpose — it is the user's own data, meant
to be found and read, and syncing it is a feature.

A config left in `~/Documents/Nudge/` by an earlier version is **moved**, once,
before it is read. Copying instead would leave the settings window writing one
file while a hand-editing user reads the other.

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
| Global hotkey | text | A string in the §5 grammar. See below. |
| Default interval (min) | text | A number, must be finite and positive; same rules as the minutes field of the popup in §3. |
| Open at login / Launch with Windows | checkbox | Registers the app to launch at session start. The label names the platform's own mechanism, because that is what tells the user whether the setting applies to them. |

#### The hotkey row

**One editable text field, and nothing else.** It holds a string in the §5
grammar, shown in this platform's spelling, with the accepted vocabulary
printed under it — the field is the only way to set a hotkey, so what may go
in it is standing information, not an error message.

A stored value that parses is re-spelled for this platform on load, so a
config written by the other build, seeded by first-run or edited by hand
reads as `⌃⇧Space` on a Mac. One that does *not* parse is shown exactly as
stored: it is the only copy of what the user typed, and the field is where
they fix it.

**There is no capture button.** One was built and removed. Capture needs to
know which keys are held *right now*, and the only thing the GUI toolkit
offers for that — its set of currently-down keys — is not reliable on macOS
while a modifier is held: measured over 965 frames of real typing, the key
dropped out of that set 25 times mid-press, for 0.08–0.63 s at a time, and
the longest unbroken stretch of one combination was 0.67 s. Any rule of the
form "held for N seconds" is unimplementable on that signal, and every
softer rule tried in its place ("first key wins", "commit on release") is
either non-deterministic or fires on a combination the user was still
assembling. The failure is not visible as an error — the row simply never
accepts anything — which is the worst shape a rule can have.

So the contract is the grammar, and the grammar alone: what the user types
is what gets bound, and §5 says exactly what may be typed.

### Saving

- **Hotkey and interval** are written to `config.json` by the `Save` button.
  Both are validated first, and if either fails the file is not touched at
  all — a banner shows the error, and for the hotkey it also states the
  accepted vocabulary, since the banner is the only place that list appears.
  A hotkey the app cannot register must not reach the file: the settings
  window is the one surface that can put one there, and a save that reported
  success would leave the next launch silently unbound.
- What is written is the **canonical spelling**, not the typed text, and the
  field is rehydrated with it — so what the user reads back is what is
  stored, and a just-saved form does not read as edited.
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
