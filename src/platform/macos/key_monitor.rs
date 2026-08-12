//! The capture instrument: an app-local `NSEvent` key-down monitor.
//!
//! This is the source the task doc measured — it needs no permission, it sees
//! the app's own keys **before** menus and text fields do, and it can swallow
//! them. The global monitor and the event tap both prompt for Input Monitoring
//! and are the wrong tool. It lives in the settings process only; the process
//! that *registers* the hotkey never reads a key.
//!
//! # What swallowing buys, structurally
//!
//! While armed the handler returns `nil` and the event stops there. So `⌘C` is
//! never turned into a copy command, `Esc` never closes the window, `Tab` never
//! moves the focus and `⌘W` never closes anything — not because a rule catches
//! them afterwards, but because nothing downstream ever sees them. The previous
//! attempt at this feature needed an extra "the toolkit already pasted"
//! event and an ordering flag purely because the toolkit had eaten the key
//! first; with the monitor there is nothing to undo, and the recorder's table
//! is exactly the six events §9 specifies.
//!
//! # What it yields
//!
//! One [`PressEdge`] per press: the `keyCode` — the physical cap, the same
//! integer `RegisterEventHotKey` takes — and the four modifiers held at that
//! instant. Never a held-key set, never a release, never a duration.
//!
//! Two things are therefore *not events at all* rather than events the table
//! has to ignore: a chord of modifiers alone and `fn` arrive as
//! `flagsChanged`, which is outside the mask; and an auto-repeat is dropped
//! here, because "a press edge is not a repeat" is a property of the event and
//! not a decision the decision table should carry.
//!
//! # Recording fixtures
//!
//! With `NUDGE_RECORD_KEYS=<path>` set, every edge is also appended to that
//! file as one NDJSON line. That is how the replay tier
//! (`tests/hotkey_replay.rs`) gets inputs a keyboard actually produced: the
//! same instrument that runs in the guest writes the stream it saw. The
//! *provenance* header of a fixture is written by the harness that pressed the
//! keys, never here — this process cannot know whether a human or a synthetic
//! `CGEvent` sent the press, and a file that claimed to know would be exactly
//! the hand-written input the acceptance rule refuses.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSEvent, NSEventMask};

use crate::recorder::{EdgeSource, PhysicalKey, PressEdge, Verdict};

use super::keycode;

/// What the handler and the row share. The handler runs on the main thread
/// inside AppKit's dispatch of the event; the row drains it from the same
/// thread on the next frame. The mutex is here so the type is `Send` for the
/// boxed `EdgeSource` the row holds, not because two threads race for it.
#[derive(Default)]
struct Shared {
    armed: AtomicBool,
    queue: Mutex<VecDeque<PressEdge>>,
}

/// A live key-down monitor. Removing it is what dropping it does.
pub struct KeyMonitor {
    shared: Arc<Shared>,
    /// The opaque token `addLocalMonitorForEventsMatchingMask:` returns.
    token: Retained<AnyObject>,
}

impl KeyMonitor {
    /// Install the monitor. **Main thread only**, and only where an
    /// `NSApplication` exists — the eframe creator closure, the same
    /// precondition `start_on_event_loop` already establishes.
    ///
    /// `ctx` is woken on every edge for the reason this module's neighbours
    /// give: a key that the toolkit never sees also never schedules a repaint,
    /// so without the wake the recorder would answer a keypress at whatever
    /// frame the *timer* next produced.
    pub fn install(ctx: egui::Context) -> Option<Self> {
        let shared = Arc::new(Shared::default());
        let handler_shared = Arc::clone(&shared);
        // `RefCell` because the block is an `Fn`: AppKit dispatches key events one
        // at a time on the main thread, so there is no second borrower to race.
        let recorder = RefCell::new(FixtureLog::from_env());

        let block = RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit hands the handler a live `NSEvent` for the
            // duration of the call, and the mask guarantees it is a key-down.
            let event: &NSEvent = unsafe { event.as_ref() };
            if !handler_shared.armed.load(Ordering::SeqCst) {
                // Idle: the field below the row is an ordinary text box and
                // the keystroke belongs to it.
                return event as *const NSEvent as *mut NSEvent;
            }
            let is_repeat = unsafe { event.isARepeat() };
            if !is_repeat {
                let ns_flags = unsafe { event.modifierFlags().bits() as u64 };
                let edge = PressEdge {
                    key: PhysicalKey(unsafe { event.keyCode() }),
                    modifiers: keycode::modifiers_from_ns_flags(ns_flags),
                };
                handler_shared.queue.lock().unwrap().push_back(edge);
                recorder.borrow_mut().append(edge, ns_flags);
                ctx.request_repaint();
            }
            // Swallowed — armed or repeating, this key is not the window's.
            std::ptr::null_mut()
        });

        // SAFETY: main thread, `NSApplication` is up, and the block outlives
        // the monitor because the monitor token retains it.
        let token = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
        };
        match token {
            Some(token) => Some(Self { shared, token }),
            None => {
                eprintln!(
                    "[nudge] could not install the key monitor; the hotkey can still be typed"
                );
                None
            }
        }
    }
}

impl Drop for KeyMonitor {
    fn drop(&mut self) {
        // SAFETY: the token came from `addLocalMonitorForEventsMatchingMask:`
        // and is removed once, on the thread that installed it.
        unsafe { NSEvent::removeMonitor(&self.token) };
    }
}

impl EdgeSource for KeyMonitor {
    fn drain(&mut self) -> Vec<PressEdge> {
        self.shared.queue.lock().unwrap().drain(..).collect()
    }

    fn set_armed(&mut self, armed: bool) {
        self.shared.armed.store(armed, Ordering::SeqCst);
        if !armed {
            // Anything queued belongs to a recording that is over. Keeping it
            // would make the next arming answer a key pressed before it.
            self.shared.queue.lock().unwrap().clear();
        }
    }

    fn classify(&self, edge: PressEdge) -> Verdict {
        keycode::classify_press(edge)
    }
}

/// The `NUDGE_RECORD_KEYS` sink. One NDJSON line per edge, appended.
struct FixtureLog(Option<std::fs::File>);

impl FixtureLog {
    fn from_env() -> Self {
        let Ok(path) = std::env::var("NUDGE_RECORD_KEYS") else {
            return Self(None);
        };
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => Self(Some(file)),
            Err(e) => {
                eprintln!("[nudge] cannot record key edges to {path}: {e}");
                Self(None)
            }
        }
    }

    /// `ns_flags` is the event's own `modifierFlags`, **unconverted**. It is
    /// recorded next to the converted nibble so the replay tier can check the
    /// conversion against what the OS actually raised — including the `fn` bit
    /// macOS sets by itself for every arrow and every F key, which is the one
    /// bit a hand-written stream would never think to carry.
    fn append(&mut self, edge: PressEdge, ns_flags: u64) {
        let Some(file) = self.0.as_mut() else {
            return;
        };
        let PhysicalKey(code) = edge.key;
        let line = format!(
            "{{\"keycode\":{code},\"modifiers\":{},\"ns_flags\":{ns_flags}}}\n",
            edge.modifiers
        );
        if let Err(e) = file.write_all(line.as_bytes()) {
            eprintln!("[nudge] cannot record a key edge: {e}");
            self.0 = None;
        }
    }
}
