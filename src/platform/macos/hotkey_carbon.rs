use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::hotkey::{self, Hotkey};

use super::keycode;
use super::symbolic;

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

type EventHandlerProcPtr = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

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

const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");

const K_EVENT_HOT_KEY_EXCLUSIVE: u32 = 1;
const K_EVENT_HOT_KEY_NO_OPTIONS: u32 = 0;

pub const EVENT_HOT_KEY_EXISTS_ERR: OSStatus = -9878;

const NUDGE_SIGNATURE: u32 = u32::from_be_bytes(*b"NDGE");
const BINDING_ID: u32 = 1;
const PROBE_ID: u32 = 2;

pub struct Registration {
    reference: EventHotKeyRef,
    hotkey: Hotkey,
}

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
    Unbindable(String),
    Exclusive,
    NoReference,
    Os(OSStatus),
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::Unbindable(key) => write!(
                f,
                "macOS cannot bind \"{key}\" — it has no key code for it (see design-spec §5)"
            ),
            RegisterError::Exclusive => {
                write!(f, "another application holds this combination exclusively")
            }
            RegisterError::NoReference => {
                write!(
                    f,
                    "RegisterEventHotKey returned success without a reference"
                )
            }
            RegisterError::Os(status) => write!(f, "OSStatus {status}"),
        }
    }
}

pub fn register(hk: &Hotkey) -> Result<Registration, RegisterError> {
    let key = carbon_key(hk).ok_or_else(|| unbindable(hk))?;
    install_press_handler();
    bind_key(hk, key, BINDING_ID, K_EVENT_HOT_KEY_NO_OPTIONS)
}

fn carbon_key(hk: &Hotkey) -> Option<(u16, u32)> {
    let code = keycode::keycode_for(hk.key.as_str())?;
    Some((code, keycode::carbon_modifiers(hk.modifiers)))
}

fn bind(hk: &Hotkey, id: u32, options: u32) -> Result<Registration, RegisterError> {
    let key = carbon_key(hk).ok_or_else(|| unbindable(hk))?;
    bind_key(hk, key, id, options)
}

fn unbindable(hk: &Hotkey) -> RegisterError {
    RegisterError::Unbindable(hk.key.as_str().to_string())
}

fn bind_key(
    hk: &Hotkey,
    (code, carbon_mods): (u16, u32),
    id: u32,
    options: u32,
) -> Result<Registration, RegisterError> {
    let reference = register_raw(code, carbon_mods, id, options)?;
    Ok(Registration {
        reference,
        hotkey: hk.clone(),
    })
}

pub fn register_exclusive(hk: &Hotkey) -> Result<Registration, RegisterError> {
    bind(hk, PROBE_ID, K_EVENT_HOT_KEY_EXCLUSIVE)
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
        0 => Err(RegisterError::NoReference),
        EVENT_HOT_KEY_EXISTS_ERR => Err(RegisterError::Exclusive),
        other => Err(RegisterError::Os(other)),
    }
}

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
    if status == 0 && fired.signature == NUDGE_SIGNATURE && fired.id == BINDING_ID {
        super::request_manual_open();
    }
    0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyStatus {
    Free,
    HeldBySystem,
    HeldExclusively,
    HeldByNudge,
    Unbindable(String),
}

impl std::fmt::Display for HotkeyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HotkeyStatus::Free => write!(f, "free as far as we can tell"),
            HotkeyStatus::HeldBySystem => write!(
                f,
                "taken by a macOS system shortcut (see System Settings › \
                 Keyboard › Keyboard Shortcuts) — it can be saved, but the \
                 system gets the keystroke first"
            ),
            HotkeyStatus::HeldExclusively => write!(
                f,
                "taken by another application — it can be saved, but that \
                 application gets the keystroke first"
            ),
            HotkeyStatus::HeldByNudge => write!(f, "already bound by the running Nudge"),
            HotkeyStatus::Unbindable(key) => RegisterError::Unbindable(key.clone()).fmt(f),
        }
    }
}

pub fn assess(hk: &Hotkey) -> HotkeyStatus {
    let Some((code, carbon_mods)) = carbon_key(hk) else {
        return HotkeyStatus::Unbindable(hk.key.as_str().to_string());
    };

    let table = symbolic::read_symbolic_hotkeys();
    if symbolic::held_by_system(&table, code, carbon_mods).is_some() {
        return HotkeyStatus::HeldBySystem;
    }

    match bind(hk, PROBE_ID, K_EVENT_HOT_KEY_EXCLUSIVE) {
        Ok(probe) => {
            drop(probe);
            HotkeyStatus::Free
        }
        Err(RegisterError::Exclusive) => {
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
        assert_eq!(K_EVENT_CLASS_KEYBOARD, 0x6B657962);
        assert_eq!(K_EVENT_PARAM_DIRECT_OBJECT, 0x2D2D2D2D);
        assert_eq!(TYPE_EVENT_HOT_KEY_ID, 0x686B6964);
        assert_eq!(NUDGE_SIGNATURE, 0x4E444745);
    }

    #[test]
    fn a_system_shortcut_is_reported_by_where_to_change_it() {
        let text = HotkeyStatus::HeldBySystem.to_string();
        assert!(text.contains("Keyboard Shortcuts"), "{text}");
        assert!(!text.contains("row"), "{text}");
    }

    #[test]
    fn a_key_without_a_macos_code_is_refused_rather_than_guessed() {
        for token in keycode::TOKENS_WITHOUT_A_KEY_CODE {
            let hk = hotkey::parse(&format!("Ctrl+{token}")).expect("must parse");
            assert_eq!(
                register(&hk).err(),
                Some(RegisterError::Unbindable((*token).to_string())),
                "{token}"
            );
            assert_eq!(
                assess(&hk),
                HotkeyStatus::Unbindable((*token).to_string()),
                "{token}"
            );
        }
    }

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
        assert_eq!(std::mem::size_of::<EventHotKeyID>(), 8);
        assert_eq!(std::mem::align_of::<EventHotKeyID>(), 4);
        assert_eq!(std::mem::size_of::<EventTypeSpec>(), 8);
    }
}
