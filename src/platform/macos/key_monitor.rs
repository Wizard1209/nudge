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

#[derive(Default)]
struct Shared {
    armed: AtomicBool,
    queue: Mutex<VecDeque<PressEdge>>,
}

pub struct KeyMonitor {
    shared: Arc<Shared>,
    token: Retained<AnyObject>,
}

impl KeyMonitor {
    pub fn install(ctx: egui::Context) -> Option<Self> {
        let shared = Arc::new(Shared::default());
        let handler_shared = Arc::clone(&shared);
        let recorder = RefCell::new(FixtureLog::from_env());

        // A local NSEvent monitor gets copies of this app's events before their dispatch, and a
        // handler that returns nil stops the dispatching of the event.
        // proof: https://developer.apple.com/documentation/appkit/nsevent/addlocalmonitorforevents(matching:handler:)
        let block = RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            let event: &NSEvent = unsafe { event.as_ref() };
            if !handler_shared.armed.load(Ordering::SeqCst) {
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
            std::ptr::null_mut()
        });

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
            self.shared.queue.lock().unwrap().clear();
        }
    }

    fn classify(&self, edge: PressEdge) -> Verdict {
        keycode::classify_press(edge)
    }
}

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

    fn append(&mut self, edge: PressEdge, ns_flags: u64) {
        let Some(file) = self.0.as_mut() else {
            return;
        };
        let PhysicalKey(code) = edge.key;
        let line = format!(
            "{{\"keycode\":{code},\"modifiers\":{},\"ns_flags\":{ns_flags}}}\n",
            edge.modifiers.bits()
        );
        if let Err(e) = file.write_all(line.as_bytes()) {
            eprintln!("[nudge] cannot record a key edge: {e}");
            self.0 = None;
        }
    }
}
