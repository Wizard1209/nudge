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
use crate::daisy::{self, DaisyFrame};
use crate::nudge_state;

/// The armed interval: when the next nudge is due and how long the interval
/// was. The duration is what the petal countdown draws against; the deadline
/// is what the wake schedule and the tooltip count down to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmedTimer {
    pub deadline: Instant,
    pub duration: Duration,
}

impl ArmedTimer {
    /// What the countdown looks like at `now`. `elapsed` is allowed to grow
    /// past `duration` so the last petal's fall can finish before the popup
    /// takes focus (spec §5).
    pub(crate) fn observe(&self, now: Instant) -> Countdown {
        let elapsed = if now >= self.deadline {
            self.duration + now.duration_since(self.deadline)
        } else {
            self.duration
                .saturating_sub(self.deadline.duration_since(now))
        };
        Countdown {
            frame: daisy::frame_at(self.duration, elapsed),
            remaining: self.deadline.saturating_duration_since(now),
        }
    }
}

/// One observation of the countdown: spec §5's timeline (`daisy::frame_at`,
/// pure and tested) applied to the wall clock. Both backends draw from this
/// and keep for themselves only the OS I/O — the status item or the Win32
/// tray — and the way they sleep.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Countdown {
    pub frame: DaisyFrame,
    /// Saturates at zero past the deadline, so [`Countdown::tooltip`] reads
    /// "now" during the final drift.
    pub remaining: Duration,
}

/// What the icon shows: petals attached, plus the drifting petal and its
/// progress in percent. Two observations with the same key draw the same
/// picture, so a backend redraws only when this changes.
pub(crate) type IconKey = (u8, Option<(u8, u16)>);

impl Countdown {
    pub(crate) fn icon_key(&self) -> IconKey {
        let drift = self
            .frame
            .drift
            .map(|d| (d.petal_index, (d.progress * 100.0) as u16));
        (self.frame.petals_remaining, drift)
    }

    /// The minute number the tooltip renders; spec §5 wants the tooltip
    /// refreshed when this changes, not every wake.
    pub(crate) fn tooltip_minutes(&self) -> u64 {
        nudge_state::displayed_minutes(self.remaining)
    }

    pub(crate) fn tooltip(&self) -> String {
        nudge_state::tooltip_for_remaining(self.remaining)
    }

    /// When to wake next. Animate while a petal is falling, otherwise sleep
    /// until the next thing that changes what the user sees — a petal
    /// dropping, the tooltip's minute count changing, or the deadline. So an
    /// idle 10-minute interval wakes ~22 times plus twelve 250 ms bursts of
    /// animation, rather than running a standing timer.
    pub(crate) fn next_wake(&self) -> Duration {
        if self.frame.drift.is_some() {
            return DRIFT_FRAME;
        }

        let to_petal = self
            .frame
            .petal_duration
            .saturating_sub(self.frame.time_since_drop);
        // `tooltip` shows `ceil(remaining / 60)`, so its wording changes when
        // `remaining` next crosses a whole minute.
        let minutes = self.tooltip_minutes();
        let to_minute = self
            .remaining
            .saturating_sub(Duration::from_secs(minutes.saturating_sub(1) * 60));

        self.remaining.min(to_petal).min(to_minute).max(MIN_WAKE)
    }
}

/// One drift frame, ~30 Hz.
const DRIFT_FRAME: Duration = Duration::from_millis(33);

/// Sleep floor, so a zero-length answer cannot turn the wake schedule into a
/// spin.
const MIN_WAKE: Duration = Duration::from_millis(20);

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

/// Disarm the timer that just fired, so one deadline fires once: every close
/// of a timer-fired popup resets the timer (`nudge_state::decide_close`), and
/// that reset is what re-arms through [`set_timer_state`]. Only `fired` is
/// cleared — the check and the clear are one step under the lock, so a timer
/// armed after the backend read it survives.
pub(crate) fn disarm_timer(fired: ArmedTimer) {
    disarm_in(&mut TIMER.lock().unwrap(), fired);
}

fn disarm_in(slot: &mut Option<ArmedTimer>, fired: ArmedTimer) {
    if *slot == Some(fired) {
        *slot = None;
    }
}

/// Disarm whatever is armed. The macOS tests' reset between cases.
#[cfg(all(test, target_os = "macos"))]
pub(crate) fn clear_timer() {
    *TIMER.lock().unwrap() = None;
}

/// Start the settings window: a second instance of this binary with
/// `--settings`. The two processes talk only through `config.json` (and the
/// OS's autostart entry). A failed spawn is logged and otherwise ignored —
/// settings are editable by hand (spec §5) — and `--settings` is
/// single-instance in its own right, so repeat clicks cannot stack windows.
///
/// What to do with the `Child` is the backend's: dropping it closes the
/// handle on Windows, while on Unix an exited child stays a zombie until
/// its parent waits.
pub(crate) fn spawn_settings_process() -> Option<std::process::Child> {
    let exe = std::env::current_exe()
        .map_err(|e| eprintln!("[nudge] could not locate own executable: {e}"))
        .ok()?;
    std::process::Command::new(exe)
        .arg("--settings")
        .spawn()
        .map_err(|e| eprintln!("[nudge] could not open settings: {e}"))
        .ok()
}

/// Park a sprite change for the app to apply while hidden (spec §10).
pub(crate) fn park_sprite(cfg: SpriteConfig) {
    *PENDING_SPRITE.lock().unwrap() = Some(cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 10-minute interval throughout: 12 petals, one every 50s, drift capped
    // at 250ms.
    const INTERVAL: Duration = Duration::from_secs(600);

    /// The observation a backend would make with `remaining` left on the
    /// clock.
    fn with_remaining(remaining: Duration) -> Countdown {
        let now = Instant::now();
        ArmedTimer {
            deadline: now + remaining,
            duration: INTERVAL,
        }
        .observe(now)
    }

    /// The observation `past` after the deadline.
    fn past_deadline(past: Duration) -> Countdown {
        let now = Instant::now();
        ArmedTimer {
            deadline: now - past,
            duration: INTERVAL,
        }
        .observe(now)
    }

    fn wake_with(remaining: Duration) -> Duration {
        with_remaining(remaining).next_wake()
    }

    #[test]
    fn observing_past_the_deadline_keeps_elapsed_growing_and_remaining_at_zero() {
        let cd = past_deadline(Duration::from_millis(100));
        assert_eq!(cd.remaining, Duration::ZERO);
        assert_eq!(cd.tooltip(), "now");
        assert!(cd.frame.drift.is_some(), "the final petal is still falling");
        assert!(!cd.frame.should_fire);
        assert!(past_deadline(Duration::from_millis(300)).frame.should_fire);
    }

    #[test]
    fn the_icon_key_changes_with_a_petal_and_its_drift_but_not_otherwise() {
        // Two instants inside the same petal's lifetime, neither drifting.
        let a = with_remaining(Duration::from_secs(400)).icon_key();
        let b = with_remaining(Duration::from_secs(390)).icon_key();
        assert_eq!(a, b);
        // A petal later.
        assert_ne!(a, with_remaining(Duration::from_secs(340)).icon_key());
        // Mid-drift carries the drifting petal and its progress.
        let drifting = with_remaining(Duration::from_millis(524_900));
        assert!(drifting.frame.drift.is_some());
        assert_ne!(drifting.icon_key().1, None);
    }

    #[test]
    fn disarming_clears_only_the_timer_that_fired() {
        // On a local slot: the process-wide one is shared with the macOS
        // backend's tests, which run alongside.
        let now = Instant::now();
        let fired = ArmedTimer {
            deadline: now,
            duration: INTERVAL,
        };
        let rearmed = ArmedTimer {
            deadline: now + INTERVAL,
            duration: INTERVAL,
        };
        let mut slot = Some(rearmed);
        disarm_in(&mut slot, fired);
        assert_eq!(slot, Some(rearmed), "a newer timer survives");
        disarm_in(&mut slot, rearmed);
        assert_eq!(slot, None);
    }

    #[test]
    fn wake_never_overshoots_the_deadline() {
        for secs in [1, 5, 59, 60, 61, 300, 599] {
            let remaining = Duration::from_secs(secs);
            assert!(
                wake_with(remaining) <= remaining,
                "wake must not sleep past the deadline (remaining {secs}s)"
            );
        }
    }

    #[test]
    fn wake_lands_on_the_next_tooltip_wording_change() {
        // 2m05s left: the tooltip flips "~3 min" → "~2 min" in 5s, the next
        // petal is further out, so the tooltip is what sets the alarm.
        assert_eq!(wake_with(Duration::from_secs(125)), Duration::from_secs(5));
    }

    #[test]
    fn wake_lands_on_the_next_petal_drop_when_that_comes_first() {
        // 3m01s left: the tooltip boundary is 61s away and the petal is 1s
        // away (419s elapsed is 49s into the 9th petal's 50s life).
        assert_eq!(wake_with(Duration::from_secs(181)), Duration::from_secs(1));
    }

    #[test]
    fn wake_animates_while_a_petal_is_falling() {
        // 100ms into a petal drop (the second, at 75 s of a 600 s interval
        // with 16 petals), inside the 250ms drift window.
        let remaining = Duration::from_millis(524_900);
        assert!(
            with_remaining(remaining).frame.drift.is_some(),
            "this is a drift frame"
        );
        assert_eq!(wake_with(remaining), DRIFT_FRAME);
    }

    #[test]
    fn wake_keeps_animating_past_the_deadline_until_the_last_petal_lands() {
        // `should_fire` waits for the final drift, so the deadline passing is
        // not the end of the animation — the popup is owed to the user 250ms
        // later, and the frames in between are the last petal falling.
        let cd = past_deadline(Duration::from_millis(200));
        assert!(!cd.frame.should_fire, "the final drift is still running");
        assert_eq!(cd.next_wake(), DRIFT_FRAME);
    }

    #[test]
    fn wake_has_a_floor_so_it_cannot_ask_for_a_zero_length_sleep() {
        // Nothing left to count down to and no drift to animate: without the
        // floor this is a request to be woken immediately, forever.
        let cd = past_deadline(INTERVAL);
        assert!(cd.frame.drift.is_none(), "long past the last drift");
        assert_eq!(cd.next_wake(), MIN_WAKE);
    }
}
