use std::ffi::c_void;

use core_foundation::array::{CFArray, CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFGetTypeID, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSRect;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut NSRect) -> bool;
}

const ON_SCREEN_ONLY: u32 = 1 << 0;
const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const NULL_WINDOW_ID: u32 = 0;

const KEY_OWNER_PID: &str = "kCGWindowOwnerPID";
const KEY_LAYER: &str = "kCGWindowLayer";
const KEY_BOUNDS: &str = "kCGWindowBounds";

/// The bounds of the frontmost application's front window — its first
/// on-screen window at the ordinary window level, the list being ordered
/// front to back — in CoreGraphics' global space: points, origin at the
/// primary display's top-left corner, y down.
///
/// Ordinary level (layer 0) rather than any, because an application's
/// menu-bar extras, tooltips and floating panels are windows too, and are
/// not what the user is looking at. An application's front entry can be a
/// strip drawn as a window of its own (iTerm2's measured 1470×68), but it
/// sits on the same display as the window it belongs to.
///
/// `None` when no application is frontmost, when it has no such window, or
/// when the list could not be read.
pub fn front_window_bounds() -> Option<NSRect> {
    let pid = unsafe {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()?
            .processIdentifier()
    };
    let array_ref = unsafe {
        CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS, NULL_WINDOW_ID)
    };
    if array_ref.is_null() {
        return None;
    }
    let array: CFArray<*const c_void> = unsafe { CFArray::wrap_under_create_rule(array_ref) };

    let key_pid = CFString::from_static_string(KEY_OWNER_PID);
    let key_layer = CFString::from_static_string(KEY_LAYER);
    let key_bounds = CFString::from_static_string(KEY_BOUNDS);

    let count = unsafe { CFArrayGetCount(array.as_concrete_TypeRef()) };
    for i in 0..count {
        let entry = unsafe { CFArrayGetValueAtIndex(array.as_concrete_TypeRef(), i) };
        if entry.is_null() {
            continue;
        }
        let dict = entry as CFDictionaryRef;
        if unsafe { CFGetTypeID(dict as CFTypeRef) }
            != core_foundation::dictionary::CFDictionary::<*const c_void, *const c_void>::type_id()
        {
            continue;
        }
        let number = |key: &CFString| -> Option<i64> {
            let value = unsafe { CFDictionaryGetValue(dict, key.as_CFTypeRef()) };
            if value.is_null() || unsafe { CFGetTypeID(value) } != CFNumber::type_id() {
                return None;
            }
            unsafe { CFNumber::wrap_under_get_rule(value as _) }.to_i64()
        };
        if number(&key_pid) != Some(i64::from(pid)) || number(&key_layer) != Some(0) {
            continue;
        }
        let bounds = unsafe { CFDictionaryGetValue(dict, key_bounds.as_CFTypeRef()) };
        if bounds.is_null() {
            continue;
        }
        let mut rect = NSRect::default();
        if !unsafe { CGRectMakeWithDictionaryRepresentation(bounds as CFDictionaryRef, &mut rect) }
        {
            continue;
        }
        return Some(rect);
    }
    None
}
