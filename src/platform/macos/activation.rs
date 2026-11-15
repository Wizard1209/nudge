//! Spec §1's keyboard hand-back: remember which application had the
//! keyboard when the popup took it, and give it back when the popup closes.
//! The remembered application is this module's alone; the backend calls
//! [`remember_front_application`] when it activates the popup and
//! [`hand_back_activation`] when it goes away.

use std::sync::Mutex;

use objc2::runtime::NSObjectProtocol;
use objc2::sel;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
};
use objc2_foundation::MainThreadMarker;

/// The application that had the keyboard when the popup took it, by pid:
/// what [`hand_back_activation`] gives it back to (spec §1). Written by
/// `focus_popup` on the first activation of a show, taken by the hide.
/// `None` while hidden, and while shown when nothing else was active at
/// the moment the popup took the keyboard.
static PREVIOUS_FRONT: Mutex<Option<i32>> = Mutex::new(None);

/// Spec §1: the popup "hands [the keyboard] back to the application that
/// had it when it closes". AppKit does not do that on its own: ordering the
/// window out leaves this app the active one with no key window, so every
/// keystroke goes nowhere until the user clicks, and the next show reads
/// nudge as the frontmost application, finds no ordinary-level window of
/// its own and falls through to the pointer's display.
///
/// Only while this app still is the active one: a Leave by focus loss means
/// the user has already put another application in front, and taking that
/// from them is the opposite of handing back. The remembered application
/// gets the activation; one that is gone, or none remembered, means hiding
/// instead, which has the system pick the next application the way ⌘H does.
pub(super) fn hand_back_activation() {
    let previous = PREVIOUS_FRONT.lock().unwrap().take();
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: plain accessors and requests on the shared application, on
    // the main thread; the running application is returned retained.
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

/// Note which application is about to lose the keyboard to the popup, for
/// [`hand_back_activation`] — once per show: the first call of a show is
/// the one made before the activation, and the retries that follow it on a
/// popup not yet key would otherwise read this app as the previous one.
pub(super) fn remember_front_application() {
    let mut previous = PREVIOUS_FRONT.lock().unwrap();
    if previous.is_some() {
        return;
    }
    // SAFETY: `sharedWorkspace` is a plain class accessor; the frontmost
    // application is returned retained, and `processIdentifier` only reads it.
    let pid = unsafe {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|front| front.processIdentifier())
    };
    if pid.is_some_and(|pid| pid != std::process::id() as i32) {
        *previous = pid;
    }
}
