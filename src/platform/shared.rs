//! The half of the seam every native backend has in common: the edge flags
//! `NudgeApp` polls, and the state it hands whoever runs the countdown — the
//! tray thread on Windows, `tick` on the main thread on macOS. Process-wide
//! statics, because the two sides are different threads (Windows) or
//! different frames (macOS) and share no object. A backend supplies one
//! thing on top: `super::wake`, called after a write it has to act on.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::config::SpriteConfig;

/// The armed interval: when the next nudge is due and how long the interval
/// was. The duration is what the petal countdown draws against; the deadline
/// is what the wake schedule and the tooltip count down to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmedTimer {
    pub deadline: Instant,
    pub duration: Duration,
}

/// `None` when no timer is armed — before the first close (spec §4 freezes
/// the timer until then) and between an expiry and the reset that follows.
static TIMER: Mutex<Option<ArmedTimer>> = Mutex::new(None);

/// Edge flags: raised by the backend when it has revealed the popup, taken
/// by `NudgeApp::ui`, which performs the one state transition for both
/// triggers.
static TRAY_CLICKED: AtomicBool = AtomicBool::new(false);
static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

/// `true` while the popup is on screen: the backend neither counts down nor
/// animates then. Starts `true` because the app opens with the launch popup.
static POPUP_VISIBLE: AtomicBool = AtomicBool::new(true);

/// Set by the `config_watcher` callback, off any backend thread, when
/// `config.json` changes on disk; taken by the backend on its next pass.
/// A flag rather than a counter: N saves before the next pass collapse into
/// one re-read, which is the correct behaviour.
static CONFIG_RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

/// A sprite change from a config reload, waiting for the app to apply it
/// while the popup is hidden (spec §10). The newest change replaces an
/// older one still waiting.
static PENDING_SPRITE: Mutex<Option<SpriteConfig>> = Mutex::new(None);

// ---------- what the app reads and writes ----------------------------------

pub fn take_tray_clicked() -> bool {
    TRAY_CLICKED.swap(false, Ordering::SeqCst)
}

pub fn take_timer_fired() -> bool {
    TIMER_FIRED.swap(false, Ordering::SeqCst)
}

/// The sprite change parked by the last reload, if any; `NudgeApp` drains it.
pub fn take_pending_sprite() -> Option<SpriteConfig> {
    PENDING_SPRITE.lock().unwrap().take()
}

/// Arm the timer. Called from `NudgeApp::hide_popup` whenever the decision
/// resets the interval; wakes the backend so the deadline is scheduled now
/// rather than at its next unrelated pass.
pub fn set_timer_state(timer: ArmedTimer) {
    // Scoped so the lock is released before `wake` — on macOS that takes a
    // different mutex and calls into egui, and nothing needs both at once.
    {
        *TIMER.lock().unwrap() = Some(timer);
    }
    super::wake();
}

/// Mirror the popup's visibility to the backend, which parks while it is up.
pub fn set_popup_visible(visible: bool) {
    POPUP_VISIBLE.store(visible, Ordering::SeqCst);
    super::wake();
}

/// Signal that `config.json` changed on disk. Idempotent, and wakes the
/// backend: the hotkey is live-reloadable, so a reload must not sit
/// unapplied until the timer happens to wake it.
pub fn request_config_reload() {
    CONFIG_RELOAD_REQUESTED.store(true, Ordering::SeqCst);
    super::wake();
}

// ---------- what the backend reads and raises ------------------------------

pub(crate) fn raise_tray_clicked() {
    TRAY_CLICKED.store(true, Ordering::SeqCst);
}

pub(crate) fn raise_timer_fired() {
    TIMER_FIRED.store(true, Ordering::SeqCst);
}

pub(crate) fn popup_visible() -> bool {
    POPUP_VISIBLE.load(Ordering::SeqCst)
}

pub(crate) fn take_config_reload_requested() -> bool {
    CONFIG_RELOAD_REQUESTED.swap(false, Ordering::SeqCst)
}

pub(crate) fn armed_timer() -> Option<ArmedTimer> {
    *TIMER.lock().unwrap()
}

/// Disarm, so one deadline fires once: every close of a timer-fired popup
/// resets the timer (`nudge_state::decide_close`), and that reset is what
/// re-arms through [`set_timer_state`].
pub(crate) fn disarm_timer() {
    *TIMER.lock().unwrap() = None;
}

/// Park a sprite change for the app to apply while hidden (spec §10).
pub(crate) fn park_sprite(cfg: SpriteConfig) {
    *PENDING_SPRITE.lock().unwrap() = Some(cfg);
}
