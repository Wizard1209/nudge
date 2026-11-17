#![cfg(target_os = "macos")]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use nudge_web::config::Config;
use nudge_web::hotkey::{self, Modifiers};
use nudge_web::platform::keycode;
use nudge_web::recorder::{EdgeSource, Input, PhysicalKey, PressEdge, Verdict};
use nudge_web::settings_app::{HotkeyRecorder, SettingsForm, apply_effect};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystreams")
}

#[derive(Debug)]
struct Provenance {
    bullet: String,
    os: String,
    layout: String,
    input: String,
    keys: String,
    harness: String,
}

const ACCEPTED_INPUT_KINDS: [&str; 2] = ["real-keyboard", "synthetic-cgevent"];

#[derive(Debug, Clone, Copy)]
struct RecordedEdge {
    edge: PressEdge,
    ns_flags: u64,
}

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
            .and_then(|bits| u8::try_from(bits).ok())
            .and_then(Modifiers::from_bits)
            .ok_or_else(|| {
                format!(
                    "{}: edge {n} has no modifiers, or a number that is not a set of the four",
                    path.display()
                )
            })?;
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
                modifiers,
            },
            ns_flags,
        });
    }
    Ok(Stream { provenance, edges })
}

struct ReplayEdgeSource {
    pending: VecDeque<PressEdge>,
    armed: bool,
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

struct Bullet {
    name: &'static str,
    saved: &'static str,
    field_after: Option<&'static str>,
    armed_after: bool,
    hint_contains: Option<&'static str>,
    edges: usize,
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
    Bullet {
        name: "cmd-c",
        saved: "Ctrl+Shift+Space",
        field_after: Some("Cmd+C"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x08),
    },
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
    Bullet {
        name: "bare-f13",
        saved: "Ctrl+Shift+Space",
        field_after: Some("F13"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x69),
    },
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
    Bullet {
        name: "russian-layout-alt-cmd-j",
        saved: "Ctrl+Shift+Space",
        field_after: Some("Alt+Cmd+J"),
        armed_after: false,
        hint_contains: None,
        edges: 1,
        first_code: Some(0x26),
    },
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
        ..Config::default()
    };
    let mut form = SettingsForm::from_config(config);
    let source = ReplayEdgeSource::new(stream.edges.iter().map(|e| e.edge).collect());
    let mut row = HotkeyRecorder::new(Some(Box::new(source)));

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

fn native(label: &str) -> String {
    hotkey::format_native(&hotkey::parse(label).expect("the expectation must parse"))
}

#[test]
fn the_recorder_has_recorded_evidence() {
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
    let dir = fixture_dir();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
        panic!(
            "{}: {e} — the fixture directory is part of the repo, and a \
                                    missing one is a lost recording, not an empty tier",
            dir.display()
        )
    });
    let mut seen = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "ndjson") {
            continue;
        }
        let stream = parse_stream(&path).unwrap_or_else(|e| panic!("{e}"));
        let expected = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(
            stream.provenance.bullet,
            expected,
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
    assert!(
        !seen.is_empty(),
        "no fixtures at all — see the ratchet above"
    );
}

#[test]
fn every_recorded_stream_replays_to_what_its_bullet_promises() {
    let dir = fixture_dir();

    for bullet in BULLETS {
        let path = dir.join(format!("{}.ndjson", bullet.name));
        if !path.exists() {
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
            stray_bits |= recorded.ns_flags & !((1 << 17) | (1 << 18) | (1 << 19) | (1 << 20));
            edges += 1;
        }
    }
    assert!(
        edges > 0,
        "no recorded edges at all — see the ratchet above"
    );
    assert_ne!(
        stray_bits, 0,
        "not one recorded edge carries a flag outside the four Carbon can bind, so the \
         dropping of `fn`, capsLock and numericPad is asserted here against nothing. The \
         arrow and F-row recordings are where those bits come from."
    );
}

#[test]
fn a_fixture_with_no_provenance_is_refused() {
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
