use std::sync::Mutex;

use objc2::runtime::NSObjectProtocol;
use objc2::sel;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
};
use objc2_foundation::MainThreadMarker;

static PREVIOUS_FRONT: Mutex<Option<i32>> = Mutex::new(None);

pub(super) fn hand_back_activation() {
    let previous = PREVIOUS_FRONT.lock().unwrap().take();
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    unsafe {
        if !app.isActive() {
            return;
        }
        let target = previous
            .and_then(|pid| NSRunningApplication::runningApplicationWithProcessIdentifier(pid))
            .filter(|target| !target.isTerminated());
        let handed = target.is_some_and(|target| {
            let options = NSApplicationActivationOptions::empty();
            // macOS 14 made activation cooperative: the active application
            // names the one it hands over to. Older systems take the plain
            // request, and do not have the selector.
            if target.respondsToSelector(sel!(activateFromApplication:options:)) {
                target.activateFromApplication_options(
                    &NSRunningApplication::currentApplication(),
                    options,
                )
            } else {
                target.activateWithOptions(options)
            }
        });
        if !handed {
            app.hide(None);
        }
    }
}

pub(super) fn remember_front_application() {
    let mut previous = PREVIOUS_FRONT.lock().unwrap();
    if previous.is_some() {
        return;
    }
    let pid = unsafe {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|front| front.processIdentifier())
    };
    if pid.is_some_and(|pid| pid != std::process::id() as i32) {
        *previous = pid;
    }
}
