//! Replay of key streams a keyboard actually produced.
//!
//! This is the fast tier of the recorder's evidence, and the whole reason it
//! can be fast: the guest suite (`tests/macos_hotkey_native.rs`) presses real
//! keys against a real desktop and, on the way, writes down every press edge
//! the monitor saw. Those files land here, and from then on the same input
//! runs on every `cargo test` in a fraction of a second.
//!
//! **A test for this feature is evidence only if its input is a recorded real
//! event stream or a real keypress.** A suite that passes against a
//! hand-written sequence the platform never produces has shipped a dead
//! recorder here twice, so the rule is mechanical rather than a habit: every
//! fixture must open with a provenance line saying which machine, which
//! layout, which physical keys and which harness produced it, and a file
//! without one is rejected instead of replayed. Writing a fixture by hand
//! means writing a lie into that line, which is a different act from being
//! careless.
//!
//! What is replayed is the real path, end to end short of the window: the
//! recorded key code → `classify_press` → the decision table → the settings
//! form's field. The one thing standing in for the OS is where the edges come
//! from, which is exactly the seam the fixture exists to fill.

#![cfg(target_os = "macos")]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use nudge_web::config::Config;
use nudge_web::hotkey;
use nudge_web::platform::keycode;
use nudge_web::recorder::{EdgeSource, Input, PhysicalKey, PressEdge, Verdict};
use nudge_web::settings_app::{HotkeyRecorder, SettingsForm, apply_effect};

// ---------------------------------------------------------------------------
// the fixture format
// ---------------------------------------------------------------------------

/// Where the guest suite drops what it recorded.
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystreams")
}

/// The first line of every fixture. Each field is required, and the test
/// refuses a file that leaves one out — this is the gate that a hand-authored
/// sequence cannot pass by looking like the others.
#[derive(Debug)]
struct Provenance {
    /// Which acceptance bullet this stream was recorded for.
    bullet: String,
    /// The OS the keys were pressed on.
    os: String,
    /// The active input source. `⌥⌘J` from a Russian layout is a different
    /// recording from `⌥⌘J` on a US one, and the point is that they agree.
    layout: String,
    /// `real-keyboard` or `synthetic-cgevent`. Nothing else counts: both are
    /// events the OS delivered, which is what the acceptance rule asks for.
    input: String,
    /// The physical keys a human would say were pressed.
    keys: String,
    /// The test that pressed them, so the recording can be reproduced.
    harness: String,
}

const ACCEPTED_INPUT_KINDS: [&str; 2] = ["real-keyboard", "synthetic-cgevent"];

/// One recorded press: the edge the monitor produced **and** the raw
/// `modifierFlags` the `NSEvent` carried before conversion.
///
/// Both are written down because the conversion between them is code that
/// ships — and it is the piece a hand-written stream cannot exercise honestly:
/// macOS raises the `fn` bit by itself for every arrow and every F key, and
/// dropping it is what keeps `⌥⌘←` the combination the user pressed.
#[derive(Debug, Clone, Copy)]
struct RecordedEdge {
    edge: PressEdge,
    ns_flags: u64,
}

/// One fixture: its provenance and the edges the monitor saw, in order.
#[derive(Debug)]
struct Stream {
    provenance: Provenance,
    edges: Vec<RecordedEdge>,
}

fn parse_stream(path: &Path) -> Result<Stream, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));

    let header = lines
        .next()
        .ok_or_else(|| format!("{}: empty fixture", path.display()))?;
    let header: serde_json::Value = serde_json::from_str(header)
        .map_err(|e| format!("{}: header is not JSON: {e}", path.display()))?;

    let field = |name: &str| -> Result<String, String> {
        header
            .get(name)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                format!(
                    "{}: the provenance line has no \"{name}\". A fixture without provenance is \
                     indistinguishable from a hand-written one, and hand-written input is what \
                     this tier exists to keep out.",
                    path.display()
                )
            })
    };
    let provenance = Provenance {
        bullet: field("bullet")?,
        os: field("os")?,
        layout: field("layout")?,
        input: field("input")?,
        keys: field("keys")?,
        harness: field("harness")?,
    };
    if !ACCEPTED_INPUT_KINDS.contains(&provenance.input.as_str()) {
        return Err(format!(
            "{}: input kind {:?} is not one the OS delivers ({:?})",
            path.display(),
            provenance.input,
            ACCEPTED_INPUT_KINDS
        ));
    }

    let mut edges = Vec::new();
    for (n, line) in lines.enumerate() {
        let edge: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| format!("{}: edge {n} is not JSON: {e}", path.display()))?;
        let code = edge
            .get("keycode")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("{}: edge {n} has no keycode", path.display()))?;
        let modifiers = edge
            .get("modifiers")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("{}: edge {n} has no modifiers", path.display()))?;
        let ns_flags = edge
            .get("ns_flags")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                format!(
                    "{}: edge {n} has no ns_flags. The raw modifierFlags are part of what the \
                     monitor recorded; without them the conversion the app ships is covered only \
                     by numbers somebody typed.",
                    path.display()
                )
            })?;
        edges.push(RecordedEdge {
            edge: PressEdge {
                key: PhysicalKey(code as u16),
                modifiers: modifiers as u8,
            },
            ns_flags,
        });
    }
    Ok(Stream { provenance, edges })
}

// ---------------------------------------------------------------------------
// the source that stands in for the keyboard
// ---------------------------------------------------------------------------

/// A key source whose edges came off a keyboard some time ago.
///
/// It is the only substitution this tier makes, and it is deliberately the
/// narrowest one available: everything downstream of `drain` — the key code,
/// the modifier nibble, the classification, the table, the field — is the code
/// the window runs.
struct ReplayEdgeSource {
    pending: VecDeque<PressEdge>,
    armed: bool,
    /// Every edge that arrived while *not* armed. The monitor would have
    /// passed those to the window untouched, so a fixture that only replays
    /// correctly with the recorder disarmed is a fixture recorded wrong.
    seen_while_idle: usize,
}

impl ReplayEdgeSource {
    fn new(edges: Vec<PressEdge>) -> Self {
        Self {
            pending: edges.into(),
            armed: false,
            seen_while_idle: 0,
        }
    }
}

impl EdgeSource for ReplayEdgeSource {
    fn drain(&mut self) -> Vec<PressEdge> {
        if !self.armed {
            self.seen_while_idle += self.pending.len();
            return Vec::new();
        }
        self.pending.drain(..).collect()
    }

    fn set_armed(&mut self, armed: bool) {
        self.armed = armed;
    }

    fn classify(&self, edge: PressEdge) -> Verdict {
        keycode::classify_press(edge)
    }
}

// ---------------------------------------------------------------------------
// what each recorded stream must do
// ---------------------------------------------------------------------------

/// One acceptance bullet, as an assertion over the stream recorded for it.
struct Bullet {
    /// The fixture's basename, without `.ndjson`, and the `bullet` its
    /// provenance line must carry.
    name: &'static str,
    /// What is in `config.json` when the recording starts — every cancelling
    /// path has to land back on exactly this.
    saved: &'static str,
    /// What the field must read afterwards. `None` means "whatever was saved",
    /// which is what a refusal and a cancel both leave behind.
    field_after: Option<&'static str>,
    /// Whether the recorder is still listening. A refusal does not disarm: it
    /// would leave a mistyped chord looking exactly like a recorded one.
    armed_after: bool,
    /// A refusal has to name the key. This is the name it must contain.
    hint_contains: Option<&'static str>,
    /// How many press edges the recording must hold, and — where the bullet is
    /// about a particular physical cap — which `kVK_*` code the first of them
    /// must carry.
    ///
    /// This is an assertion about the *fixture*, not about the table, and it
    /// is what keeps the two empty streams from being vacuous: `⌥⌘` alone and
    /// `fn` alone replay to "still armed, field unchanged" under any
    /// implementation whatsoever, so what has to be pinned there is that the
    /// OS delivered nothing, which is a fact about the file.
    edges: usize,
    /// `kVK_*`, spelled out. The guest posts these as literals too, so a
    /// wrong row in `keycode.rs` cannot be pressed and read back through
    /// itself into agreement.
    first_code: Option<u16>,
}

const BULLETS: &[Bullet] = &[
    Bullet {
        name: "alt-cmd-j",
        saved: "Ctrl+Shift+Space",
        field_after: Some("Alt+Cmd+J"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x26),
    },
    Bullet {
        name: "alt-cmd-left",
        saved: "Ctrl+Shift+Space",
        field_after: Some("Alt+Cmd+Left"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x7B),
    },
    Bullet {
        name: "shift-slash",
        saved: "Ctrl+Shift+Space",
        field_after: Some("Shift+/"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x2C),
    },
    // ⌘C reaches the recorder as three plain facts — key code 8, the Command
    // bit, nothing else. It was only ever a clipboard command because the
    // toolkit saw it first, and the monitor swallows it before anything can.
    Bullet {
        name: "cmd-c",
        saved: "Ctrl+Shift+Space",
        field_after: Some("Cmd+C"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x08),
    },
    // Keypad 5 and the digit row's 5 are different keys with different codes.
    // The toolkit folds them together; this is where that fold would show.
    Bullet {
        name: "keypad-5",
        saved: "Ctrl+Shift+Space",
        field_after: None,
        armed_after: true,
        hint_contains: Some("Keypad 5"),
        edges: 1,
        first_code: Some(0x57),
    },
    Bullet {
        name: "bare-letter",
        saved: "Ctrl+Shift+Space",
        field_after: None,
        armed_after: true,
        hint_contains: Some("J"),
        edges: 1,
        first_code: Some(0x26),
    },
    // The F row types nothing, which is what makes a bare F13 a usable
    // shortcut and the one exception to the modifier rule.
    Bullet {
        name: "bare-f13",
        saved: "Ctrl+Shift+Space",
        field_after: Some("F13"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x69),
    },
    // These two are recorded as *empty* streams, and that is the assertion:
    // modifiers alone and `fn` arrive as flagsChanged, which the monitor's
    // mask excludes, so they are not events at all rather than events the
    // table has to ignore.
    Bullet {
        name: "modifiers-only",
        saved: "Ctrl+Shift+Space",
        field_after: None,
        armed_after: true,
        hint_contains: None,
        edges: 0,
        first_code: None,
    },
    Bullet {
        name: "fn-only",
        saved: "Ctrl+Shift+Space",
        field_after: None,
        armed_after: true,
        hint_contains: None,
        edges: 0,
        first_code: None,
    },
    // A key is a position, not the character it produces. Same cap, same code,
    // same combination — recorded with the layout switched so the claim is
    // observed rather than asserted.
    Bullet {
        name: "russian-layout-alt-cmd-j",
        saved: "Ctrl+Shift+Space",
        field_after: Some("Alt+Cmd+J"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x26),
    },
    // A hotkey needs a modifier, so a lone Esc can never be one and the
    // keystroke is free to mean "back out".
    Bullet {
        name: "bare-escape",
        saved: "Alt+Cmd+J",
        field_after: None,
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x35),
    },
];

/// Replay one stream into an armed recorder and report what the row shows.
struct Outcome {
    field: String,
    armed: bool,
    hint: Option<String>,
}

fn replay(stream: &Stream, saved: &str) -> Outcome {
    let config = Config {
        hotkey: saved.to_string(),
        default_interval_minutes: 10.0,
        autostart: false,
    };
    let mut form = SettingsForm::from_config(config);
    let source = ReplayEdgeSource::new(stream.edges.iter().map(|e| e.edge).collect());
    let mut row = HotkeyRecorder::new(Some(Box::new(source)));

    // The Record button. Everything after this is the recording.
    row.feed(&Input::Record);
    for effect in row.pump(true) {
        apply_effect(&effect, &mut form);
    }
    Outcome {
        field: form.hotkey.clone(),
        armed: row.is_armed(),
        hint: row.hint().map(str::to_string),
    }
}

/// The §5 spelling of a label, which is what the field must read.
fn native(label: &str) -> String {
    hotkey::format_native(&hotkey::parse(label).expect("the expectation must parse"))
}

// ---------------------------------------------------------------------------
// the tests
// ---------------------------------------------------------------------------

#[test]
fn the_recorder_has_recorded_evidence() {
    // The ratchet, and the reason it is its own test: with no fixtures at all
    // every assertion below is about an empty set, and an empty set satisfies
    // anything. This feature has shipped a dead recorder twice; the difference
    // this time has to be a red test rather than a note in a document.
    //
    // Every guest run rewrites the whole set, so a partial one means a guest
    // test did not run — the same failure, caught the same way.
    let dir = fixture_dir();
    let missing: Vec<&str> = BULLETS
        .iter()
        .map(|b| b.name)
        .filter(|name| !dir.join(format!("{name}.ndjson")).exists())
        .collect();
    assert!(
        missing.is_empty(),
        "no recorded key stream for {missing:?}. This tier replays what a real \
         keyboard produced; without the recording there is no evidence at all. \
         Record it: `SUITE=macos_hotkey_native scripts/native-tests-macos.sh` \
         (see tests/fixtures/keystreams/README.md)."
    );
}

#[test]
fn every_fixture_carries_its_provenance() {
    // Run over the directory rather than the expectation list, so a stray file
    // cannot sit there unread and a future bullet cannot be added without one.
    let dir = fixture_dir();
    let entries = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e} — the fixture directory is part of the repo, and a \
                                    missing one is a lost recording, not an empty tier", dir.display()));
    let mut seen = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "ndjson") {
            continue;
        }
        let stream = parse_stream(&path).unwrap_or_else(|e| panic!("{e}"));
        let expected = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(
            stream.provenance.bullet, expected,
            "{} says it was recorded for a different bullet",
            path.display()
        );
        assert!(
            stream.provenance.os.contains("macOS"),
            "{}: recorded on {:?}, which is not this platform",
            path.display(),
            stream.provenance.os
        );
        assert!(
            BULLETS.iter().any(|b| b.name == expected),
            "{} is not one of the acceptance bullets. A stream nothing asserts \
             against is a recording nobody reads.",
            path.display()
        );
        seen.push(expected.to_string());
    }
    assert!(!seen.is_empty(), "no fixtures at all — see the ratchet above");
}

#[test]
fn every_recorded_stream_replays_to_what_its_bullet_promises() {
    let dir = fixture_dir();

    for bullet in BULLETS {
        let path = dir.join(format!("{}.ndjson", bullet.name));
        if !path.exists() {
            // `the_recorder_has_recorded_evidence` is where a missing stream
            // is reported; failing again here would only bury it.
            continue;
        }
        let stream = parse_stream(&path).unwrap_or_else(|e| panic!("{e}"));
        let context = format!(
            "{} — {} pressed on {} ({}), {}",
            bullet.name,
            stream.provenance.keys,
            stream.provenance.layout,
            stream.provenance.input,
            stream.provenance.harness
        );

        // What the OS delivered, before anything interprets it. For the two
        // empty streams this is the whole assertion: they replay to "still
        // armed, field unchanged" under any implementation, so the claim being
        // kept is that no press edge arrived.
        assert_eq!(
            stream.edges.len(),
            bullet.edges,
            "the recording for {context} holds {} edges, not {}: {:?}",
            stream.edges.len(),
            bullet.edges,
            stream.edges
        );
        if let Some(code) = bullet.first_code {
            assert_eq!(
                stream.edges[0].edge.key,
                PhysicalKey(code),
                "{context}: the cap the OS reported is not the one this bullet is about"
            );
        }

        let outcome = replay(&stream, bullet.saved);
        let expected_field = bullet
            .field_after
            .map_or_else(|| native(bullet.saved), native);
        assert_eq!(outcome.field, expected_field, "field after {context}");
        assert_eq!(outcome.armed, bullet.armed_after, "armed after {context}");
        if let Some(name) = bullet.hint_contains {
            let hint = outcome.hint.unwrap_or_default();
            assert!(
                hint.contains(name),
                "the refusal must name the key ({name}) after {context}: {hint:?}"
            );
        }
    }
}

#[test]
fn the_recorded_flags_convert_to_the_recorded_modifiers() {
    // `modifiers_from_ns_flags` is the one piece of the capture path a stream
    // of already-converted nibbles would leave uncovered — and it is where the
    // `fn` bit lives. macOS raises that bit by itself for the arrows and the F
    // row, so honouring it would turn `⌥⌘←` into a combination the user cannot
    // type. Here the raw flags come off a real event and the conversion is run
    // over them.
    let dir = fixture_dir();
    let mut stray_bits = 0u64;
    let mut edges = 0usize;
    for bullet in BULLETS {
        let path = dir.join(format!("{}.ndjson", bullet.name));
        if !path.exists() {
            continue;
        }
        let stream = parse_stream(&path).unwrap_or_else(|e| panic!("{e}"));
        for (n, recorded) in stream.edges.iter().enumerate() {
            assert_eq!(
                keycode::modifiers_from_ns_flags(recorded.ns_flags),
                recorded.edge.modifiers,
                "{}: edge {n} was recorded with flags {:#x}, which do not convert to the \
                 modifiers recorded next to them",
                bullet.name,
                recorded.ns_flags
            );
            // Everything outside the four Carbon can bind.
            stray_bits |= recorded.ns_flags & !((1 << 17) | (1 << 18) | (1 << 19) | (1 << 20));
            edges += 1;
        }
    }
    assert!(edges > 0, "no recorded edges at all — see the ratchet above");
    assert_ne!(
        stray_bits, 0,
        "not one recorded edge carries a flag outside the four Carbon can bind, so the \
         dropping of `fn`, capsLock and numericPad is asserted here against nothing. The \
         arrow and F-row recordings are where those bits come from."
    );
}

#[test]
fn a_fixture_with_no_provenance_is_refused() {
    // The gate itself, exercised on a file written for the purpose. This is
    // the one place in the tier where a hand-written stream is the subject
    // rather than the evidence.
    let dir = std::env::temp_dir().join(format!("nudge-replay-gate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let bare = dir.join("bare.ndjson");
    std::fs::write(&bare, "{\"keycode\":38,\"modifiers\":10}\n").expect("write");
    let err = parse_stream(&bare).expect_err("a stream with no provenance line must be refused");
    assert!(err.contains("provenance"), "{err}");

    let unattributed = dir.join("unattributed.ndjson");
    std::fs::write(
        &unattributed,
        "{\"bullet\":\"x\",\"os\":\"macOS 26.5.2\",\"layout\":\"US\",\"input\":\"imagined\",\
         \"keys\":\"⌥⌘J\",\"harness\":\"none\"}\n",
    )
    .expect("write");
    let err = parse_stream(&unattributed).expect_err("an input kind the OS cannot deliver");
    assert!(err.contains("imagined"), "{err}");

    let _ = std::fs::remove_dir_all(&dir);
}
