//! Registering the global hotkey, by calling the OS directly.
//!
//! `RegisterEventHotKey` is what `global-hotkey` called on our behalf, and the
//! crate is gone for two reasons the task doc measured rather than assumed:
//! it cannot reach the exclusive registration mode, and on macOS its error
//! type never reports a conflict for a real key — a check written against it
//! works on Windows and silently never fires here.
//!
//! # What the call does and does not do (measured, macOS 26.5)
//!
//! - It **validates no key code**. A nonsense value registers and returns
//!   `noErr`, and the binding then never fires. This is why
//!   [`super::keycode::keycode_for`] returning `None` has to be a refusal here
//!   rather than a guess.
//! - It **does not report a combination another process holds**. `⌘Space`,
//!   `⌘Tab` and the screenshot shortcuts all register with no complaint.
//!   Nothing in this module derives "somebody else has it" from the outcome of
//!   the call; that question is [`assess`]'s, and it uses different calls.
//!
//! # No header, still a stable ABI
//!
//! The macOS 26 SDK ships no Carbon Event Manager declarations at all — the
//! symbols are still exported by `HIToolbox` (they are in the `.tbd`), but the
//! headers that declared them are gone. So the `extern` block below is the
//! declaration, written from the ABI: `OSStatus` is `SInt32`, `ItemCount` and
//! `ByteCount` are `unsigned long`, `OptionBits` is `UInt32`, and
//! `EventHotKeyID` is two 32-bit words passed by value.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::hotkey::{self, Hotkey};

use super::keycode;
use super::symbolic;

// ---------- the ABI --------------------------------------------------------

#[allow(non_camel_case_types)]
type OSStatus = i32;
type EventRef = *mut c_void;
type EventTargetRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type EventHandlerProcPtr =
    extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;

    fn RegisterEventHotKey(
        in_hot_key_code: u32,
        in_hot_key_modifiers: u32,
        in_hot_key_id: EventHotKeyID,
        in_target: EventTargetRef,
        in_options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;

    fn UnregisterEventHotKey(in_hot_key: EventHotKeyRef) -> OSStatus;

    fn InstallEventHandler(
        in_target: EventTargetRef,
        in_handler: EventHandlerProcPtr,
        in_num_types: u64,
        in_list: *const EventTypeSpec,
        in_user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;

    fn GetEventParameter(
        in_event: EventRef,
        in_name: u32,
        in_desired_type: u32,
        out_actual_type: *mut u32,
        in_buffer_size: u64,
        out_actual_size: *mut u64,
        out_data: *mut c_void,
    ) -> OSStatus;
}

/// `'keyb'`, the Carbon event class hotkeys arrive on.
const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
/// `kEventHotKeyPressed`. The release edge (`6`) is deliberately **not**
/// subscribed: Carbon delivers both, and acting on the release would re-open a
/// popup the user just dismissed with the same chord. Not filtering it out —
/// never asking for it.
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
/// `kEventParamDirectObject` (`'----'`) as a `typeEventHotKeyID` (`'hkid'`) —
/// how a handler learns *which* hotkey fired.
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");

/// `kEventHotKeyExclusive`. Asks the OS to refuse the registration if another
/// *exclusive* holder already has the combination — the only arbitration
/// macOS offers, and half of what [`assess`] reports.
const K_EVENT_HOT_KEY_EXCLUSIVE: u32 = 1;
const K_EVENT_HOT_KEY_NO_OPTIONS: u32 = 0;

/// `eventHotKeyExistsErr` — an exclusive registration refused because someone
/// else holds the combination exclusively.
pub const EVENT_HOT_KEY_EXISTS_ERR: OSStatus = -9878;

/// Our four-character signature, and the two ids under it: the binding the app
/// actually runs on, and the throwaway one [`assess`]'s probe uses. Separate
/// so the press handler can ignore a probe's registration, and so a probe that
/// collides with our own binding is recognisable as ours.
const NUDGE_SIGNATURE: u32 = u32::from_be_bytes(*b"NDGE");
const BINDING_ID: u32 = 1;
const PROBE_ID: u32 = 2;

// ---------- registration ---------------------------------------------------

/// A live registration. Unregisters on drop, which is what makes the live
/// config reload a matter of dropping one value and building another.
pub struct Registration {
    reference: EventHotKeyRef,
    /// What was bound, for the log line at reload time.
    hotkey: Hotkey,
}

// The `EventHotKeyRef` is an opaque pointer into HIToolbox's own bookkeeping;
// it is never dereferenced here and only ever unregistered from the thread
// that owns the run loop. `Send` is asserted so `Registration` can live in the
// same struct as the rest of the reload state.
unsafe impl Send for Registration {}

impl Drop for Registration {
    fn drop(&mut self) {
        let status = unsafe { UnregisterEventHotKey(self.reference) };
        if status != 0 {
            eprintln!(
                "[nudge] unregistering hotkey \"{}\" failed: OSStatus {status}",
                hotkey::format_native(&self.hotkey)
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// §5 accepts the combination but this platform has no key code for it —
    /// `F21`–`F24`. Refused rather than guessed: a guessed code registers
    /// happily and then never fires.
    Unmappable(String),
    /// An exclusive holder already has it. Only reachable from the probe;
    /// the app's own binding is never exclusive.
    Exclusive,
    Os(OSStatus),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::Unmappable(key) => write!(
                f,
                "macOS has no key code for \"{key}\" — see design-spec §5"
            ),
            RegisterError::Exclusive => {
                write!(f, "another application holds this combination exclusively")
            }
            RegisterError::Os(status) => write!(f, "OSStatus {status}"),
        }
    }
}

/// Bind `hk` system-wide, non-exclusively.
///
/// Non-exclusive on purpose: exclusivity is a detection tool
/// ([`assess`]), and taking it here would let a detection concern cost the
/// user their hotkey.
pub fn register(hk: &Hotkey) -> Result<Registration, RegisterError> {
    let code = keycode::keycode_for(hk.key.as_str())
        .ok_or_else(|| RegisterError::Unmappable(hk.key.as_str().to_string()))?;
    install_press_handler();
    let reference = register_raw(
        code,
        keycode::carbon_modifiers(hk.modifiers),
        BINDING_ID,
        K_EVENT_HOT_KEY_NO_OPTIONS,
    )?;
    Ok(Registration {
        reference,
        hotkey: hk.clone(),
    })
}

/// Register `hk` **exclusively** and keep it. Not what the app binds with —
/// exclusivity is a detection tool, and taking it for the real binding would
/// let a detection concern cost the user their hotkey. Its one caller is
/// `examples/hotkey_probe.rs`, which has to *hold* a combination while another
/// process asks [`assess`] about it: that is the only way to find out whether
/// the exclusive mode arbitrates across processes at all, and the answer is
/// what decides whether source 2 of the conflict report is worth having.
#[allow(dead_code)] // the probe is an example binary; the app never calls this
pub fn register_exclusive(hk: &Hotkey) -> Result<Registration, RegisterError> {
    let code = keycode::keycode_for(hk.key.as_str())
        .ok_or_else(|| RegisterError::Unmappable(hk.key.as_str().to_string()))?;
    let reference = register_raw(
        code,
        keycode::carbon_modifiers(hk.modifiers),
        PROBE_ID,
        K_EVENT_HOT_KEY_EXCLUSIVE,
    )?;
    Ok(Registration {
        reference,
        hotkey: hk.clone(),
    })
}

fn register_raw(
    code: u16,
    carbon_mods: u32,
    id: u32,
    options: u32,
) -> Result<EventHotKeyRef, RegisterError> {
    let mut reference: EventHotKeyRef = std::ptr::null_mut();
    let status = unsafe {
        RegisterEventHotKey(
            code as u32,
            carbon_mods,
            EventHotKeyID {
                signature: NUDGE_SIGNATURE,
                id,
            },
            GetApplicationEventTarget(),
            options,
            &mut reference,
        )
    };
    match status {
        0 if !reference.is_null() => Ok(reference),
        0 => Err(RegisterError::Os(0)),
        EVENT_HOT_KEY_EXISTS_ERR => Err(RegisterError::Exclusive),
        other => Err(RegisterError::Os(other)),
    }
}

/// Install the one `kEventHotKeyPressed` handler on the application event
/// target. Idempotent — the handler outlives every individual binding, so a
/// live reload re-registers the hotkey and not this.
fn install_press_handler() {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spec = EventTypeSpec {
        event_class: K_EVENT_CLASS_KEYBOARD,
        event_kind: K_EVENT_HOT_KEY_PRESSED,
    };
    let mut handler: EventHandlerRef = std::ptr::null_mut();
    let status = unsafe {
        InstallEventHandler(
            GetApplicationEventTarget(),
            hotkey_pressed,
            1,
            &spec,
            std::ptr::null_mut(),
            &mut handler,
        )
    };
    if status != 0 {
        eprintln!("[nudge] could not install the hotkey handler: OSStatus {status}");
    }
}

/// The handler. It holds no policy — it raises the same flag the status-item
/// click raises and wakes the event loop, and [`super::tick`] decides what that
/// means. Same shape as every other handler in this module, for the reason its
/// header gives.
extern "C" fn hotkey_pressed(
    _call: EventHandlerCallRef,
    event: EventRef,
    _user_data: *mut c_void,
) -> OSStatus {
    let mut fired = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    let status = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>() as u64,
            std::ptr::null_mut(),
            (&raw mut fired).cast::<c_void>(),
        )
    };
    // A probe's registration is live for microseconds, but it *is* live, and
    // opening the popup because a conflict check was running would be a bug
    // with a very confusing report.
    if status == 0 && fired.signature == NUDGE_SIGNATURE && fired.id == BINDING_ID {
        super::request_manual_open();
    }
    // `noErr`: the event is ours and handled. Returning `eventNotHandledErr`
    // would pass it on, and nothing else is listening for it.
    0
}

// ---------- conflict, best-effort, from two sources ------------------------

/// What we can say about a combination without binding it for real.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyStatus {
    /// Neither source found a holder. Not a promise: this is the answer both
    /// sources give when they simply cannot see the holder.
    Free,
    /// macOS's own shortcut table has it — `⌘Space`, `⌘Tab`, the screenshot
    /// shortcuts. Registration reports nothing about these; the hotkey would
    /// bind and never fire. Carries the table row, which is all macOS gives:
    /// the dictionaries hold no identifier.
    HeldBySystem(i64),
    /// Another application registered it exclusively.
    HeldExclusively,
    /// The running Nudge instance holds it. Distinguished from a foreign
    /// conflict because the settings window is a separate process and would
    /// otherwise report the app's own binding as somebody else's.
    HeldByNudge,
    /// §5 accepts it but this platform cannot bind it at all.
    Unregistrable(String),
}

impl std::fmt::Display for HotkeyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HotkeyStatus::Free => write!(f, "free as far as we can tell"),
            HotkeyStatus::HeldBySystem(row) => write!(
                f,
                "taken by a macOS system shortcut (system table row {row}) — \
                 it can be saved, but the system gets the keystroke first"
            ),
            HotkeyStatus::HeldExclusively => write!(
                f,
                "taken by another application — it can be saved, but that \
                 application gets the keystroke first"
            ),
            HotkeyStatus::HeldByNudge => write!(f, "already bound by the running Nudge"),
            HotkeyStatus::Unregistrable(key) => {
                write!(f, "macOS cannot bind \"{key}\" (see design-spec §5)")
            }
        }
    }
}

/// Ask both sources who holds `hk`, and say which one answered.
///
/// Deliberately read-mostly and deliberately separate from [`register`]: the
/// system table is a pure read, and the exclusive probe unregisters
/// immediately, so nothing here can leave a binding behind or take one away.
///
/// # What each source can and cannot see (measured — `examples/hotkey_probe`)
///
/// The system table cannot see another application's registration, and the
/// exclusive probe cannot see the system's own shortcuts. That much was the
/// reason for having two. The probe is also **narrower than it looks**:
/// exclusive mode arbitrates against any registration inside the *same*
/// process, but across processes only against another *exclusive* holder — and
/// almost nothing registers exclusively. So the probe is the source that
/// rarely fires and is certain when it does, while the system table is the one
/// that catches the case a user actually meets (`⌘Space`).
///
/// The same measurement is why [`HotkeyStatus::HeldByNudge`] exists and why
/// only the process that binds can ever report it: our own non-exclusive
/// binding refuses our own probe, and is invisible to anybody else's.
pub fn assess(hk: &Hotkey) -> HotkeyStatus {
    let Some(code) = keycode::keycode_for(hk.key.as_str()) else {
        return HotkeyStatus::Unregistrable(hk.key.as_str().to_string());
    };
    let carbon_mods = keycode::carbon_modifiers(hk.modifiers);

    // Source 1: the system's own table. First because it is the one that
    // costs nothing and catches ⌘Space.
    let table = symbolic::read_symbolic_hotkeys();
    if let Some(row) = symbolic::held_by_system(&table, code, carbon_mods) {
        return HotkeyStatus::HeldBySystem(row);
    }

    // Source 2: an exclusive registration, immediately dropped.
    match register_raw(code, carbon_mods, PROBE_ID, K_EVENT_HOT_KEY_EXCLUSIVE) {
        Ok(reference) => {
            let status = unsafe { UnregisterEventHotKey(reference) };
            if status != 0 {
                eprintln!("[nudge] conflict probe could not unregister: OSStatus {status}");
            }
            HotkeyStatus::Free
        }
        Err(RegisterError::Exclusive) => {
            // Measured: an exclusive probe *does* collide with this process's
            // own non-exclusive binding. So inside the process that binds, the
            // likeliest holder is the app itself, and saying so is the
            // difference between a warning and a lie.
            if super::holds_this_hotkey(hk) {
                HotkeyStatus::HeldByNudge
            } else {
                HotkeyStatus::HeldExclusively
            }
        }
        Err(e) => {
            eprintln!("[nudge] conflict probe failed: {e}");
            HotkeyStatus::Free
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_character_codes_are_the_carbon_ones() {
        // Written as byte literals so they read as the codes Apple documents;
        // asserted as numbers so a typo in the literal is caught rather than
        // shipped into an extern call.
        assert_eq!(K_EVENT_CLASS_KEYBOARD, 0x6B657962);
        assert_eq!(K_EVENT_PARAM_DIRECT_OBJECT, 0x2D2D2D2D);
        assert_eq!(TYPE_EVENT_HOT_KEY_ID, 0x686B6964);
        assert_eq!(NUDGE_SIGNATURE, 0x4E444745);
    }

    #[test]
    fn a_key_without_a_macos_code_is_refused_rather_than_guessed() {
        // F21..F24 parse (Windows can bind them) and macOS has no kVK for
        // them. Registration validates nothing, so a guessed code would
        // return success and never fire — this is the one place that says no.
        for token in keycode::TOKENS_WITHOUT_A_KEY_CODE {
            let hk = hotkey::parse(&format!("Ctrl+{token}")).expect("must parse");
            assert_eq!(
                register(&hk).err(),
                Some(RegisterError::Unmappable((*token).to_string())),
                "{token}"
            );
            assert_eq!(
                assess(&hk),
                HotkeyStatus::Unregistrable((*token).to_string()),
                "{token}"
            );
        }
    }

    /// The measurement `HeldByNudge` exists for, asserted against the real OS
    /// rather than quoted from a probe run.
    ///
    /// `#[ignore]` because it takes a machine-wide combination for the length
    /// of the test — `⌃⌥⇧⌘F19`, four modifiers on a key most keyboards do not
    /// have, so even a leaked registration takes nothing pressable. Run it
    /// with `cargo test --lib -- --ignored exclusive`.
    #[test]
    #[ignore = "registers a real system-wide hotkey"]
    fn an_exclusive_probe_collides_with_our_own_non_exclusive_binding() {
        let hk = hotkey::parse("Ctrl+Alt+Shift+Cmd+F19").unwrap();
        assert_eq!(
            assess(&hk),
            HotkeyStatus::Free,
            "nothing should hold the probe combination before we do"
        );
        let held = register(&hk).expect("the probe combination must bind");
        super::super::note_own_hotkey(Some(hk.clone()));
        assert_eq!(
            assess(&hk),
            HotkeyStatus::HeldByNudge,
            "the exclusive probe sees our own non-exclusive binding — which is \
             the whole reason a process has to be able to recognise itself"
        );
        drop(held);
        super::super::note_own_hotkey(None);
        assert_eq!(assess(&hk), HotkeyStatus::Free, "dropping unregisters");
    }

    #[test]
    fn the_event_hot_key_id_is_two_words_by_value() {
        // The extern block is the only declaration of this struct in the
        // process — the SDK ships none — so its shape is asserted here.
        assert_eq!(std::mem::size_of::<EventHotKeyID>(), 8);
        assert_eq!(std::mem::align_of::<EventHotKeyID>(), 4);
        assert_eq!(std::mem::size_of::<EventTypeSpec>(), 8);
    }
}
