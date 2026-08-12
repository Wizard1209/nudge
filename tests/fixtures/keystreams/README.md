# Recorded key streams

Every `*.ndjson` here is a list of press edges a real macOS session delivered
to Nudge's key monitor, with a provenance line saying where it came from.
`tests/hotkey_replay.rs` replays them on every `cargo test`; that is the only
reason the recorder has fast tests at all.

**Nothing in this directory may be written by hand.** The acceptance rule for
the hotkey recorder is that a test counts only if its input is a recorded real
event stream or a real keypress — a suite that passes against an invented
sequence the platform never produces has shipped a dead recorder here twice.
The replay tier enforces the rule mechanically: a file without a complete
provenance line is refused rather than replayed, so writing a fixture by hand
means writing a lie into that line, which is a different act from being
careless.

## Where they come from

```sh
SUITE=macos_hotkey_native scripts/native-tests-macos.sh
```

`tests/macos_hotkey_native.rs` presses real keys against the guest's desktop
while the settings binary runs with `NUDGE_RECORD_KEYS` pointing at a file.
The script copies the results back here. Every run rewrites all of them, which
is what keeps the fast tier and the slow tier from drifting apart.

## When they must be re-recorded

**Touching `src/platform/macos/key_monitor.rs` or
`src/platform/macos/keycode.rs` invalidates every file here.** Those two decide
what an edge *is* — which events the mask lets through, and which key code and
modifier bits the edge carries — so a stream recorded before the change is a
recording of the old capture path. Replaying it then proves the old path still
works, which is not the question.

Nothing enforces this. The replay tier checks that the files exist, carry
provenance and still produce the expected verdicts; it cannot tell a stale
recording from a fresh one, because a stale one is a real recording of a real
keyboard. Re-record with the command above, review the diff, and commit the
streams with the change that caused them.

## Format

One JSON object per line. The first is the provenance, the rest are edges in
the order the monitor saw them.

```
{"bullet":"alt-cmd-j","os":"macOS 26.5.2 (25F84)","layout":"com.apple.keylayout.US","input":"synthetic-cgevent","keys":"⌥⌘J","harness":"macos_hotkey_native::records_the_spelled_combinations"}
{"keycode":38,"modifiers":10,"ns_flags":1573160}
```

| Field | Meaning |
|---|---|
| `bullet` | Which acceptance bullet this was recorded for. Must equal the filename. |
| `os` | The system the keys were pressed on. |
| `layout` | The active input source, **read** at record time via `TISCopyCurrentKeyboardInputSource` rather than named by the harness. `⌥⌘J` from a Russian layout is a different recording from `⌥⌘J` on a US one, and the point is that they agree — a literal here would be the harness asserting the one claim in the header nobody can check. |
| `input` | `real-keyboard` or `synthetic-cgevent`. Nothing else is accepted: both are events the OS delivered. |
| `keys` | What a person would say was pressed. |
| `harness` | The test that pressed them, so a recording can be reproduced. |

An edge is `{"keycode": <kVK_*>, "modifiers": <mask>, "ns_flags": <raw>}` — the
physical cap, the four modifiers held at that instant, and the `NSEvent`'s own
`modifierFlags` **before** conversion. The mask is `crate::hotkey`'s: `1`
Control, `2` Option, `4` Shift, `8` Command.

The raw flags are there because the conversion between the two columns is code
that ships, and a stream of already-converted nibbles is exactly what leaves it
uncovered: macOS raises the `fn` bit by itself for every arrow and every F key,
and dropping it is what keeps `⌥⌘←` the combination the user pressed. The
replay tier runs `modifiers_from_ns_flags` over the recorded flags and requires
the two columns to agree — and requires at least one recording to carry a bit
outside the four, so the drop is asserted against something the OS really
raised.

An **empty** stream (a provenance line and nothing else) is a real result and
two bullets are exactly that: `⌥⌘` alone and `fn` alone arrive as
`flagsChanged`, which the monitor's mask excludes, so they are not events the
recorder has to ignore — they are not events.

An empty stream is also what a monitor that never installed would leave
behind, so the recording is not the whole evidence for those two. In the
guest, each null is preceded *in the same process* by a chord that must land:
`⌥⌘J` for the modifier case, and `⌥⌘J` with `fn` also held for the `fn` case —
the second showing `CGEventFlagSecondaryFn` is postable and does arrive. Only
then is the stream truncated and the modifiers held alone. The replay tier
carries its half of that by asserting the edge *count*, so a file that grew
edges fails rather than replaying to the same answer.
