#![cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::SpriteConfig;
use crate::daisy::{self, DaisyFrame};
use crate::nudge_state;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmedTimer {
    pub deadline: Instant,
    pub duration: Duration,
}

impl ArmedTimer {
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct Countdown {
    pub frame: DaisyFrame,
    pub remaining: Duration,
}

pub(crate) type IconKey = (u8, Option<(u8, u16)>);

impl Countdown {
    pub(crate) fn icon_key(&self) -> IconKey {
        let drift = self
            .frame
            .drift
            .map(|d| (d.petal_index, (d.progress * 100.0) as u16));
        (self.frame.petals_remaining, drift)
    }

    pub(crate) fn tooltip_minutes(&self) -> u64 {
        nudge_state::displayed_minutes(self.remaining)
    }

    pub(crate) fn tooltip(&self) -> String {
        nudge_state::tooltip_for_remaining(self.remaining)
    }

    pub(crate) fn next_wake(&self) -> Duration {
        if self.frame.drift.is_some() {
            return DRIFT_FRAME;
        }

        let to_petal = self
            .frame
            .petal_duration
            .saturating_sub(self.frame.time_since_drop);
        let minutes = self.tooltip_minutes();
        let to_minute = self
            .remaining
            .saturating_sub(Duration::from_secs(minutes.saturating_sub(1) * 60));

        self.remaining.min(to_petal).min(to_minute).max(MIN_WAKE)
    }
}

const DRIFT_FRAME: Duration = Duration::from_millis(33);

const MIN_WAKE: Duration = Duration::from_millis(20);

static TIMER: Mutex<Option<ArmedTimer>> = Mutex::new(None);

static TRAY_CLICKED: AtomicBool = AtomicBool::new(false);
static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

static POPUP_VISIBLE: AtomicBool = AtomicBool::new(false);

static CONFIG_RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

static PENDING_SPRITE: Mutex<Option<SpriteConfig>> = Mutex::new(None);

static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn take_tray_clicked() -> bool {
    TRAY_CLICKED.swap(false, Ordering::SeqCst)
}

pub fn take_timer_fired() -> bool {
    TIMER_FIRED.swap(false, Ordering::SeqCst)
}

pub fn take_pending_sprite() -> Option<SpriteConfig> {
    PENDING_SPRITE.lock().unwrap().take()
}

pub fn set_timer_state(timer: ArmedTimer) {
    {
        *TIMER.lock().unwrap() = Some(timer);
    }
    super::wake();
}

pub fn set_popup_visible(visible: bool) {
    POPUP_VISIBLE.store(visible, Ordering::SeqCst);
    super::wake();
}

pub fn request_config_reload() {
    CONFIG_RELOAD_REQUESTED.store(true, Ordering::SeqCst);
    super::wake();
}

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

pub fn armed_timer() -> Option<ArmedTimer> {
    *TIMER.lock().unwrap()
}

pub(crate) fn disarm_timer(fired: ArmedTimer) {
    disarm_in(&mut TIMER.lock().unwrap(), fired);
}

fn disarm_in(slot: &mut Option<ArmedTimer>, fired: ArmedTimer) {
    if *slot == Some(fired) {
        *slot = None;
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn clear_timer() {
    *TIMER.lock().unwrap() = None;
}

pub(crate) fn spawn_settings_process(args: &[String]) -> Option<std::process::Child> {
    let exe = std::env::current_exe()
        .map_err(|e| eprintln!("[nudge] could not locate own executable: {e}"))
        .ok()?;
    std::process::Command::new(exe)
        .args(settings_args(CONFIG_PATH.get().map(PathBuf::as_path)))
        .args(args)
        .spawn()
        .map_err(|e| eprintln!("[nudge] could not open settings: {e}"))
        .ok()
}

pub(crate) fn record_config_path(path: PathBuf) {
    let _ = CONFIG_PATH.set(path);
}

fn settings_args(config_path: Option<&Path>) -> Vec<OsString> {
    let mut args = vec![OsString::from(crate::settings_app::SETTINGS_ARG)];
    if let Some(path) = config_path {
        args.extend(["--config".into(), path.into()]);
    }
    args
}

pub(crate) fn park_sprite(cfg: SpriteConfig) {
    *PENDING_SPRITE.lock().unwrap() = Some(cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_process_opens_the_file_this_process_runs_on() {
        let in_use = Path::new("/Users/Jane Doe/Documents/Nudge/config.json");
        let args: Vec<String> = settings_args(Some(in_use))
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();

        assert!(crate::settings_app::parse_settings_arg(&args), "{args:?}");
        let file = crate::config::ConfigFile::from_args_or(&args, || {
            panic!("the settings process resolved the config itself: {args:?}")
        });
        assert_eq!(file.path(), in_use);
    }

    const INTERVAL: Duration = Duration::from_secs(600);

    fn with_remaining(remaining: Duration) -> Countdown {
        let now = Instant::now();
        ArmedTimer {
            deadline: now + remaining,
            duration: INTERVAL,
        }
        .observe(now)
    }

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
        let a = with_remaining(Duration::from_secs(400)).icon_key();
        let b = with_remaining(Duration::from_secs(390)).icon_key();
        assert_eq!(a, b);
        assert_ne!(a, with_remaining(Duration::from_secs(340)).icon_key());
        let drifting = with_remaining(Duration::from_millis(524_900));
        assert!(drifting.frame.drift.is_some());
        assert_ne!(drifting.icon_key().1, None);
    }

    #[test]
    fn disarming_clears_only_the_timer_that_fired() {
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
    fn the_longest_interval_arms_and_counts_down() {
        let duration = crate::nudge_state::parse_interval("1000000").unwrap();
        let now = Instant::now();
        let countdown = ArmedTimer {
            deadline: now + duration,
            duration,
        }
        .observe(now);
        assert_eq!(countdown.remaining, duration);
        assert!(countdown.next_wake() <= Duration::from_secs(60));
        assert_eq!(countdown.tooltip(), "~1000000 min");
    }

    #[test]
    fn wake_lands_on_the_next_tooltip_wording_change() {
        assert_eq!(wake_with(Duration::from_secs(125)), Duration::from_secs(5));
    }

    #[test]
    fn wake_lands_on_the_next_petal_drop_when_that_comes_first() {
        assert_eq!(wake_with(Duration::from_secs(181)), Duration::from_secs(1));
    }

    #[test]
    fn wake_animates_while_a_petal_is_falling() {
        let remaining = Duration::from_millis(524_900);
        assert!(
            with_remaining(remaining).frame.drift.is_some(),
            "this is a drift frame"
        );
        assert_eq!(wake_with(remaining), DRIFT_FRAME);
    }

    #[test]
    fn wake_keeps_animating_past_the_deadline_until_the_last_petal_lands() {
        let cd = past_deadline(Duration::from_millis(200));
        assert!(!cd.frame.should_fire, "the final drift is still running");
        assert_eq!(cd.next_wake(), DRIFT_FRAME);
    }

    #[test]
    fn wake_has_a_floor_so_it_cannot_ask_for_a_zero_length_sleep() {
        let cd = past_deadline(INTERVAL);
        assert!(cd.frame.drift.is_none(), "long past the last drift");
        assert_eq!(cd.next_wake(), MIN_WAKE);
    }
}
