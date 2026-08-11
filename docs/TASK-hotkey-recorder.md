# Task — build the hotkey recorder

Self-contained brief. Everything needed is in this repo; nothing depends on
the conversation that produced it.

## What to build

`docs/design-spec.md` §9 → *The recorder*. Two states, four events, eight
cells. Read that section as the contract; do not restate it here and do not
re-derive it.

The rest of the settings window is done and passing. The hotkey **text
field** works, is tested, and is not being replaced — the recorder is a
second way in, not a substitute.

## Order of work

Non-negotiable, in this order:

1. **Tests first, from §9 alone.** One test per cell of the transition
   table, plus the acceptance list at the end of §9. Write them against the
   pure decision function, not against a live `egui::Context`. They must
   fail before there is an implementation.
2. **Then the implementation**, until those tests pass. Nothing else.
3. **Then re-run the existing suites and fix what moved.** Expect
   `tests/kittest_card.rs` to notice a changed widget count in the settings
   row.

## The trap this feature has

Two previous attempts shipped with green unit tests and did not work at all.
§9's closing subsection has the measurement; the operational rule is:

> A test for this feature is evidence only if its input is a recorded real
> session or a real keypress.

So a green suite is **not** the finish line. The finish line is the
acceptance list in §9, exercised from a real keyboard on macOS.

To watch what the toolkit actually delivers, log the key events as they
arrive and press the keys by hand — a synthetic `CGEvent` and a
programmatic `.click()` both produce a cleaner stream than a human hand, and
a disabled button swallows a programmatic click without error, so neither
substitutes for pressing the keys.

## Where things are

| | |
|---|---|
| Contract | `docs/design-spec.md` §9 → *The recorder* |
| Hotkey grammar (what may be bound) | `docs/design-spec.md` §5 |
| Settings window | `src/settings_app.rs` |
| Grammar + `hotkey_from_egui` + `format_native` | `src/hotkey.rs` |
| Registration, and what the OS will accept | `src/platform/macos.rs` |
| Widget-level tests for this window | `tests/kittest_card.rs` |

Run: `cargo test`, `cargo clippy --all-targets -- -D warnings`, and the same
clippy for `--target x86_64-pc-windows-gnu`. All three are green right now;
keep them that way.

Build and drive the window by hand with:

```sh
cargo build --bin nudge && ./target/debug/nudge --settings
```

It reads the real config (`~/Library/Application Support/Nudge/config.json`
on macOS). Pass `--config <path>` to point it at a throwaway file instead.

## Done

- Every cell of §9's table has a test.
- The §9 acceptance list passes from a real keyboard.
- `cargo test` and both clippy targets green.
- README's Settings section describes both ways to set a hotkey.
- §9 loses its "specified, not built" banner.
