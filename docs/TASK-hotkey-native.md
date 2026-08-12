# Task — read and register hotkeys through the OS, not the toolkit

Nudge has one system-wide hotkey (§5 of `docs/design-spec.md`). It can be
typed into the settings field. It cannot be captured from the keyboard: that
part of §9 is a stub, and this document is what fills it.

## What to build

**One identifier, from the key going down to the registration call.**

The window toolkit does not hand out that identifier. It folds keypad keys
onto their main-row twins, converts `⌘X` / `⌘C` / `⌘V` into clipboard
commands before any window sees a key, and reports a key that depends on the
active layout. None of that comes from the OS — it is the toolkit serving a
text editor, which is the common case and not ours. Undoing it takes a
translation table, and that table has to agree by hand with the one used for
registration. Two hand-written tables that must agree is the defect this task
removes.

Read the key from the same layer that registers it, and there is one table:
platform key code ↔ §5 token, used in both directions. On macOS the key code
the event carries is the key code the registration call takes, so nothing
sits between them at all.

## What the platform actually does

Measured on macOS 26.5, by probe. Do not re-derive these; they are the facts
the decisions below rest on.

- Registration **does not validate the key code** — nonsense values succeed.
- Registration **does not report a combination another process holds**. It
  returns success. `⌘Space`, `⌘Tab` and the screenshot shortcuts all register
  with no complaint. Detecting a conflict needs a different call than the one
  that registers.
- Registration **accepts a bare key**, which then belongs to us on the whole
  machine.
- The keypad, the arrows and `F13`–`F20` all register. Media keys are not
  reachable at all.
- An app-local key monitor needs **no permission**, sees the app's own keys
  before menus and text fields do, and can swallow them. The global monitor
  and the event tap both need a permission prompt and are the wrong tool.

**Nothing on Windows has been measured.** The documentation says
`RegisterHotKey` does arbitrate across processes and reports an
already-registered error, that `F12` is reserved for the debugger at all
times, and that some OS defaults can be silently overridden anyway. Measure
before relying on any of it.

## Decisions

1. **Registration is direct**, not through `global-hotkey`. That crate cannot
   reach the exclusive registration mode, and on macOS it never reports a
   conflict for a real key — a check written against its error type works on
   Windows and silently never fires on macOS.
2. **Conflict detection is best-effort, from two sources**, and says which
   kind it found: the exclusive registration mode catches another exclusive
   holder, and the system's own shortcut table (`CopySymbolicHotKeys`) covers
   what that misses. Neither alone is enough.
3. **At least one modifier, except `F1`–`F24`** (§5, already enforced by the
   parser).
4. **The keypad stays out of the grammar.** It registers fine; the reason is
   vocabulary size, and §5 now says so.
5. **A hotkey follows the physical key on both platforms.** macOS gets this
   free. Windows registers by a layout-dependent code, so the stored value
   has to be the physical one and the binding has to be re-made when the
   layout changes.

## The decision table

Two states, no clock. Everything that decides belongs in one pure function
taking a state and an input and returning the next state plus an effect; it
must not touch the platform, the window, or the event loop. **Whether a key
is bindable is decided outside it and handed in already answered.**

| | Idle | Armed |
|---|---|---|
| **Record** | → Armed | — |
| **Press** — bindable | — | → Idle, field := the combination, canonically spelled |
| **Press** — bare `Esc` | — | → Idle, field := the saved hotkey |
| **Press** — not bindable | — | → **Armed**, with a hint naming the key |
| **Cancel** | — | → Idle, field := the saved hotkey |
| **Focus lost** | — | → Idle, field := the saved hotkey |

Every cell is stated; no guards and no conditions outside the table. What
follows from it, rather than being added to it: a chord of modifiers alone is
not an event, so holding `⌥⌘` neither accepts nor refuses; an unbindable key
does not disarm, because disarming would leave a mistyped chord looking like
a recorded one; bare `Esc` cancels because a hotkey needs a modifier, so a
lone `Esc` can never be one; every cancelling path reads `config.json`, so
the recorder has nothing to remember when it arms; focus loss cancels because
*which* shortcut took the focus is not observable.

The only keyboard input an armed recorder accepts is a **press edge** — one
press of a non-modifier key, carrying the modifiers held at that instant. Not
the set of currently-held keys, not a release, not a duration, not a repeat.

## Order of work

1. macOS: capture, behind the existing settings field.
2. macOS: registration, replacing `global-hotkey`.
3. macOS: conflict reporting.
4. Windows: the same three, after its facts are measured.
5. §9 of the spec regains its recorder section, written against what shipped.

## Acceptance

From a real keyboard, on each platform. **A test for this feature is evidence
only if its input is a recorded real event stream or a real keypress** — a
suite that passes against a hand-written input sequence the platform never
produces has already shipped a dead recorder here twice.

- `⌥⌘J`, `⌥⌘←` and `⇧/` land in the field spelled the way §5 writes them.
- `⌘C` records as `⌘C` and does not paste.
- Keypad `5` is refused by name, and is not mistaken for the digit row.
- A bare letter is refused by name; a bare `F13` records.
- `⌥⌘` alone changes nothing and shows no error, and neither does `fn`.
- With a Russian layout active, the cap that types `о` still records `⌥⌘J`.
- Bare `Esc` puts the saved hotkey back, and so does `⌘Tab`.
- Pressing the app's own current hotkey opens the popup and ends the
  recording cancelled — the running instance holds it system-wide while the
  settings window is a separate process.
- `⌘Space` is reported as taken by the system rather than accepted silently.
- The captured value survives `Save` and a relaunch, and the app logs no
  error.
- On Windows: switching the keyboard layout does not move the shortcut to a
  different physical key.

## The Electron prototype follows the same grammar

§5 is the contract both implementations follow, and the prototype's parser
(`electron/src/shared/hotkey.ts`) is behind it: it knows letters, digits,
`F1`–`F24`, `Space`, `Enter`, `Tab`, `Escape` and `Backspace`, and nothing
else. The arrows, the navigation cluster and the symbol caps that §5 gained
are absent — `parseHotkey("Ctrl+Home")` returns an unknown-token error. Its
canonical spelling and its display strings are behind by the same amount.

The "at least one modifier, except `F1`–`F24`" rule *is* implemented there,
with tests, but **those tests have never been run in this checkout**: there
is no `package-lock.json` under `electron/` and no installed dependencies, so
the suite cannot start. Installing them writes a lock file into the
repository. Until that happens the prototype's parser has been verified only
by transpiling the module and exercising it directly.

## What must not come back

- A translation table between the toolkit's key type and ours.
- Reading the toolkit's set of currently-held keys.
- Any claim that a combination held by another application is detected by the
  registration call succeeding or failing. It is not, on macOS.
