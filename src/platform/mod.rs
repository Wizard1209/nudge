pub(crate) mod shared;
pub use shared::{
    ArmedTimer, armed_timer, request_config_reload, set_timer_state, take_pending_sprite,
    take_timer_fired, take_tray_clicked,
};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod fallback;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub use self::fallback::*;
