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
- On macOS the popup appears on whichever Space is current, full-screen
  ones included; Mission Control and the click-wallpaper-to-reveal-desktop
  gesture leave it where it is. It takes the keyboard when shown even if
  another application is activating at the same moment.
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
  its boundary. Anything drawn inside the card stays well clear of the
  hairline: a second curve running next to it at a different radius reads as
  a misaligned double outline at the corners.
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
- The active (focused) row shows a subtle rounded tint around its text: a
  field inside the row, inset 10pt from the row's sides and 5pt from its top
  and bottom, corner radius 8. It reads as the text's own input box, not as
  a second edge of the card (§2). No borders, outlines, or color accents.
- The minutes field accepts integers and decimals (e.g. `0.1` = 6 seconds, for
  testing).
- What happens to the fields on close is decided by the §4 transition table
  and stated nowhere else.

## 4. Behavior

The popup's whole behaviour is one state machine with **three states** and
**four events**. Everything below is the transition table and the definitions
its cells rely on; there are no rules outside it.

The states are named for **what the app owes the user**, not for what is on
screen, because that is what decides how a close behaves:

| State | Popup | Timer | Meaning |
|---|---|---|---|
| **Waiting** | hidden | running | Nothing is being asked. |
| **Asking·Timer** | visible | overdue | The popup was opened *because the interval elapsed*. The deadline is already in the past. |
| **Asking·Manual** | visible | running | The user opened the popup themselves. The deadline is still ahead. |

`Asking·Timer` is the initial state: the first launch shows the popup as if
the interval had elapsed. It is not a special case, it is where the machine
starts.

### Events

| Event | Raised by |
|---|---|
| **Deadline** | The interval elapses. |
| **Summon** | Tray/menu-bar left click, the `Show Nudge` menu item, or the global hotkey (§5). Three triggers, one event — they are indistinguishable downstream. |
| **Submit** | `Enter`. |
| **Leave** | `Esc`, a click outside the card, or the window losing focus **after having held it**. Three triggers, one event: the transition table treats them identically. |

The focus trigger carries its condition in its own definition rather than as
a guard: if this open of the popup never received the keyboard — a
full-screen app refused to yield foreground — then losing focus is not
something that can happen to it, and `Esc`, `Enter` and a click outside the
card remain the ways out.

### Transitions

Cells marked — are events that cannot arrive in that state, for the reason
given under the table.

| | **Deadline** | **Summon** | **Submit** | **Leave** |
|---|---|---|---|---|
| **Waiting** | → Asking·Timer | → Asking·Manual | — | — |
| **Asking·Timer** | — | — | → Waiting · *close effects* | → Waiting · **timer := field** · text kept |
| **Asking·Manual** | — | — | → Waiting · *close effects* | → Waiting · **timer untouched** · text kept |

- **Submit's close effects**, the same from both Asking states: write a
  journal line **iff** `doing` or `bullshit` is non-empty after trimming; set
  the timer to the minutes field; clear `doing` and `bullshit`.
- **Leave differs by state** because a timer-opened popup's deadline is
  already past: not setting a new one means it reappears on the next frame.
- **Deadline cannot arrive while the popup is visible** — the timer parks for
  the duration of the show, so an interval that elapses behind an open popup
  is still overdue when it closes and fires then. That is why a `Leave` from
  `Asking·Manual` can be followed at once by `Deadline`.
- **Summon while visible is dropped**, not queued.
- **The minutes field is not state.** It survives every transition, including
  `Submit`, and changes only when edited.

### Failed submits stay put

Both are **self-transitions**: the popup stays open, an error is shown, and
the timer is untouched — a nudge whose answer was lost must not count as
answered.

| Failure | Condition |
|---|---|
| Invalid interval | The minutes field is a number that is ≤ 0 or not finite. Empty, whitespace or non-numeric text is not a failure: it is not an explicit number, so it falls back to 10 minutes. |
| Journal write failed | The OS (or browser storage) refused the append. |

`Leave` has no failure path: it never writes and never validates.

## 5. Tray (native builds only)

The tray icon is a live indicator of time-to-next-nudge: it both shows the
app is running and "ripens" instead of a numeric countdown.

### Icon as an object

- Daisy (Bellis perennis): 16 petals around a bright yellow center. Each
  petal is a strip that runs out from under the centre: narrowest where it
  leaves the disc, widest about three fifths of the way out, closed by a
  pointed-oval tip. Petal and centre meet with no gap between them. The white
  petal base softens into a warm pink at the very tips — the daisy's
  signature, distinguishing it from a generic field chamomile.
- The geometry is the table in `docs/daisy/daisy.json`: the disc radius, the
  petal width and reach on a 64-unit canvas, and for every petal its angle,
  length, width and width profile, so the small irregularities of a real
  head are part of the icon and the same in every render. The table is
  baked by `docs/daisy/constructor.html` from a petal profile traced on
  Bellis perennis photographs plus a seeded, dialled-down spread; changing
  the icon means baking a new table there, not editing a renderer. Both
  implementations read the table and draw it at the host's pixel size with
  supersampled edges; no bitmap or vector file is shipped.
- Just the daisy and its center. No frame, background, shadow, digits, or
  any text inside the icon.
- The palette is the only place in the app with an accent color (pink tip +
  yellow center). The card and pill stay monochrome.
- On macOS the daisy is drawn at the menu bar's icon size, 18 pt, with one
  bitmap per display density the bar comes in (18 px for 1x, 36 px for 2x),
  each drawn at that density's pixel size and declared 18 pt in one image, so
  nothing between the renderer and the screen resamples it. On Windows it is
  a single 64 px bitmap that the notification area scales to the icon size
  (16 px at 100 % DPI).

### Link to the timer

- The flower "counts down" the current interval, dropping petal by petal.
- At the moment the timer is rearmed, all 16 petals are present.
- Over the interval exactly 16 petals fall, evenly spaced (one per
  `interval / 16`).
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

key        = letter | digit | function | named | arrow | navigation | symbol ;
letter     = "A" … "Z" ;
digit      = "0" … "9" ;
function   = "F1" … "F24" ;
named      = "Space"                    | "␣"
           | "Enter"  | "Return"        | "↩" | "⏎" | "⌤"
           | "Tab"                      | "⇥"
           | "Esc"    | "Escape"        | "⎋"
           | "Backspace"                | "⌫" ;
arrow      = "Left"                     | "←"
           | "Right"                    | "→"
           | "Up"                       | "↑"
           | "Down"                     | "↓" ;
navigation = "Home"                     | "↖"
           | "End"                      | "↘"
           | "PageUp"  | "PgUp"         | "⇞"
           | "PageDown"| "PgDn"         | "⇟"
           | "Insert"  | "Ins"
           | "Delete"  | "Del"          | "⌦" ;
symbol     = "-" | "=" | "[" | "]" | "\" | ";" | "'" | "," | "." | "/" | "`" ;
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
6. **A symbol is named by its unshifted legend.** The cap carrying `/` and
   `?` is `/`, so `⇧/` and `?` are one combination with one spelling, `⇧/`.
   `+` is only the separator, which is why the cap next to Backspace is `=`:
   `Ctrl+=`, never `Ctrl++`.
7. **A key is a position, not the character it produces.** `⌥⌘J` names the
   cap where `J` sits on a US layout; with a Russian layout active that cap
   types `о` and the same hotkey still fires. This is what the OS registers,
   and it is why switching layouts does not silently unbind a shortcut.
8. **Anything else is an error** naming the offending token.

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
- **At least one modifier is required, except for `F1`–`F24`.** The
  registration call accepts a bare key — measured: a lone `J` registers on
  macOS and returns success — and it then takes that key away from every
  application on the machine. The F-row is the exception because it types
  nothing, which is what makes a bare `F13` a usable shortcut.
- **`F1`–`F12` need `fn` held on a Mac** unless *Use F1, F2, etc. as
  standard function keys* is on. The vocabulary line offers `F1`–`F20`
  there and `F1`–`F24` on Windows: `F21`–`F24` parse on both, but whether
  a Mac registers them has never been measured.
- **The numeric keypad is not in the grammar.** Its keys carry their own
  codes and register perfectly well; leaving them out is a choice about the
  size of the vocabulary, not a limit of the platform. Adding them means
  adding a parallel name for every digit and operator.
- **`Insert` has no cap on an Apple keyboard**, and `Delete` (`⌦`, forward
  delete) is a different key from `Backspace` (`⌫`), whatever the Mac
  keycap says. Both register; only one of them is reachable on a laptop.

#### When it cannot be applied

If the config file is absent, a default is written on first launch. A
combination that cannot be registered is not fatal, and **does not fall back
to the default**: the app runs **without a hotkey**, says so on stderr, and
stays reachable through the tray icon. Silently binding something else would
take over a shortcut the user never asked for while hiding which one failed.
Startup and live reload (§9) follow the same rule.

**A combination another application already holds is not one of those
cases.** On macOS registration succeeds anyway — measured: the same
combination registers from two processes and both calls return success, and
`⌘Space` registers with no complaint at all. Nothing is reported because
nothing failed; the hotkey simply never fires, because the other holder gets
the keystroke first.

Detecting it needs different calls than the one that registers, and macOS has
two, neither sufficient alone: its own shortcut table, and an exclusive
registration probe. Both are read, the answer names which one found the
holder, and the app logs one line on stderr when it binds a combination
somebody else has. It is a warning, not a refusal — a taken combination is
still the one the user asked for. §9 says what the settings window does with
it; `docs/TASK-hotkey-native.md` records what each source can and cannot see.

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

The field always holds a string in the §5 grammar, shown in this platform's
spelling, with the accepted vocabulary printed under it — this is how a
hotkey is set, so what may go in it is standing information, not an error
message.

A stored value that parses is re-spelled for this platform on load, so a
config written by the other build, seeded by first-run or edited by hand
reads as `⌃⇧Space` on a Mac. One that does *not* parse is shown exactly as
stored: it is the only copy of what the user typed, and the field is where
they fix it.

#### Capturing a hotkey from the keyboard

Next to the field is a `Record` button — on macOS. Windows has no recorder
and shows no button, because none of its platform facts have been measured
(`docs/TASK-hotkey-native.md`); a button that cannot record is worse than no
button.

The recorder has **two states and no clock**. Everything that decides is one
transition table, and every cell of it is stated:

| | Idle | Armed |
|---|---|---|
| **Record** | → Armed | — |
| **Press** — bindable | — | → Idle, field := the combination, canonically spelled |
| **Press** — bare `Esc` | — | → Idle, field := the saved hotkey |
| **Press** — not bindable | — | → **Armed**, with a hint naming the key |
| **Cancel** | — | → Idle, field := the saved hotkey |
| **Focus lost** | — | → Idle, field := the saved hotkey |

What follows from the table rather than being added to it:

- **A chord of modifiers alone is not an event.** Holding `⌥⌘` neither accepts
  nor refuses, and neither does `fn`. They are not keys the recorder ignores;
  they never arrive.
- **An unbindable key does not disarm.** Disarming would leave a mistyped
  chord looking exactly like a recorded one. The hint names the key —
  `Keypad 5`, `J` — and the next press still lands.
- **Bare `Esc` cancels** because a hotkey needs a modifier, so a lone `Esc`
  can never be one. `⌘Esc` records like any other combination.
- **Every cancelling path reads `config.json`**, so the recorder has nothing
  to remember when it arms — and a typed-but-unsaved edit goes with the
  cancel, because cancelling has one meaning.
- **Focus loss cancels** because *which* shortcut took the focus is not
  observable: macOS consumes `⌘Tab` and `⌘Space` itself.

While armed the row asks for the combination — `Press modifiers plus one key.
Esc cancels.` — in place of both the field and the vocabulary line under it.
The field goes because a text box under that prompt reads as somewhere to
type; it is not cleared, and a cancel puts the *saved* value in it rather than
whatever was typed before `Record`.

The only keyboard input an armed recorder accepts is a **press edge** — one
press of a non-modifier key, carrying the modifiers held at that instant. Not
the set of currently-held keys, not a release, not a duration, not a repeat.
Those edges are read from the OS, at the same layer that registers the
hotkey, so the key code that arrives is the key code that gets bound and
nothing sits between them. Three things follow that a toolkit-level reader
cannot give: the keypad is a distinct key rather than a fold onto the digit
row, `⌘C` is a combination rather than a copy command, and what is captured is
the physical cap (§5 rule 7) — with a Russian layout active, the cap that
types `о` still records `⌥⌘J`. While armed the
keys are swallowed, so `⌘C` does not copy, `Esc` does not close the window and
`Tab` does not move the focus.

A hotkey the running app already holds is not recordable, and does not need
to be: the settings window is a separate process, the running instance owns
the combination system-wide, and pressing it opens the popup. The popup taking
the focus is what ends the recording.

#### When somebody else already holds the combination

Save reports it and saves anyway. §5 blocks the save on a combination the OS
**cannot bind**; one that is merely *taken* is a different thing — it binds
fine and simply never fires, because the other holder gets the keystroke
first — and the user is the one who decides whether that matters.

The report is best-effort and says which source found the holder, because
neither source alone can see everything (measured; see
`docs/TASK-hotkey-native.md`):

- **The system's own shortcut table** catches `⌘Space`, `⌘Tab` and the
  screenshot shortcuts. This is the source that fires in practice.
- **An exclusive registration probe** catches another application that took
  the combination exclusively. Almost nothing does, so it rarely fires — but
  when it does the answer is certain.

Neither can see an ordinary registration made by another application, and the
report never claims otherwise. Nothing anywhere derives "somebody else holds
it" from the registration call succeeding or failing: on macOS it succeeds
either way.

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
