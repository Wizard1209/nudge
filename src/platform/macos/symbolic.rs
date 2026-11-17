use std::ffi::c_void;

use core_foundation::array::{CFArray, CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation::base::{CFGetTypeID, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn CopySymbolicHotKeys(out_hot_keys: *mut CFArrayRef) -> i32;
}

const KEY_CODE: &str = "kHISymbolicHotKeyCode";
const KEY_MODIFIERS: &str = "kHISymbolicHotKeyModifiers";
const KEY_ENABLED: &str = "kHISymbolicHotKeyEnabled";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolicHotkey {
    pub row: i64,
    pub code: i64,
    pub carbon_modifiers: i64,
    pub enabled: bool,
}

const CARBON_MODIFIER_MASK: i64 = (1 << 8) | (1 << 9) | (1 << 11) | (1 << 12);

pub fn read_symbolic_hotkeys() -> Vec<SymbolicHotkey> {
    let mut array_ref: CFArrayRef = std::ptr::null();
    let status = unsafe { CopySymbolicHotKeys(&mut array_ref) };
    if status != 0 || array_ref.is_null() {
        eprintln!(
            "[nudge] could not read the system shortcut table (OSStatus {status}); \
             system conflicts will not be reported"
        );
        return Vec::new();
    }
    let array: CFArray<*const c_void> = unsafe { CFArray::wrap_under_create_rule(array_ref) };

    let key_code = CFString::from_static_string(KEY_CODE);
    let key_modifiers = CFString::from_static_string(KEY_MODIFIERS);
    let key_enabled = CFString::from_static_string(KEY_ENABLED);

    let count = unsafe { CFArrayGetCount(array.as_concrete_TypeRef()) };
    let mut out = Vec::with_capacity(count.max(0) as usize);
    let mut unparsed = 0usize;
    for row in 0..count {
        let entry = unsafe { CFArrayGetValueAtIndex(array.as_concrete_TypeRef(), row) };
        if entry.is_null() {
            unparsed += 1;
            continue;
        }
        let dict = entry as CFDictionaryRef;
        let (Some(code), Some(carbon_modifiers)) =
            (dict_i64(dict, &key_code), dict_i64(dict, &key_modifiers))
        else {
            unparsed += 1;
            continue;
        };
        out.push(SymbolicHotkey {
            row: row as i64,
            code,
            carbon_modifiers,
            enabled: dict_bool(dict, &key_enabled).unwrap_or(true),
        });
    }
    if unparsed > 0 {
        eprintln!(
            "[nudge] {unparsed} of {count} system shortcut entries were not in the \
             expected shape and were skipped"
        );
    }
    out
}

fn dict_value(dict: CFDictionaryRef, key: &CFString) -> Option<CFTypeRef> {
    let value = unsafe { CFDictionaryGetValue(dict, key.as_concrete_TypeRef() as *const c_void) };
    if value.is_null() {
        None
    } else {
        Some(value as CFTypeRef)
    }
}

fn dict_i64(dict: CFDictionaryRef, key: &CFString) -> Option<i64> {
    let value = dict_value(dict, key)?;
    if unsafe { CFGetTypeID(value) } != CFNumber::type_id() {
        return None;
    }
    let number = unsafe { CFNumber::wrap_under_get_rule(value as _) };
    number.to_i64()
}

fn dict_bool(dict: CFDictionaryRef, key: &CFString) -> Option<bool> {
    let value = dict_value(dict, key)?;
    if unsafe { CFGetTypeID(value) } != CFBoolean::type_id() {
        return dict_i64(dict, key).map(|n| n != 0);
    }
    Some(unsafe { CFBoolean::wrap_under_get_rule(value as _) }.into())
}

pub fn held_by_system(table: &[SymbolicHotkey], code: u16, carbon_mods: u32) -> Option<i64> {
    let wanted = carbon_mods as i64 & CARBON_MODIFIER_MASK;
    table
        .iter()
        .find(|entry| {
            entry.enabled
                && entry.code == code as i64
                && entry.carbon_modifiers & CARBON_MODIFIER_MASK == wanted
        })
        .map(|entry| entry.row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey;
    use crate::platform::macos::keycode;

    const CMD: u32 = 1 << 8;
    const SHIFT: u32 = 1 << 9;

    fn entry(row: i64, code: i64, mods: i64, enabled: bool) -> SymbolicHotkey {
        SymbolicHotkey {
            row,
            code,
            carbon_modifiers: mods,
            enabled,
        }
    }

    #[test]
    fn an_enabled_row_with_the_same_code_and_modifiers_holds_it() {
        let space = keycode::keycode_for("SPACE").unwrap();
        let table = [entry(0, space as i64, CMD as i64, true)];
        assert_eq!(held_by_system(&table, space, CMD), Some(0));
    }

    #[test]
    fn a_disabled_row_holds_nothing() {
        let space = keycode::keycode_for("SPACE").unwrap();
        let table = [entry(3, space as i64, CMD as i64, false)];
        assert_eq!(held_by_system(&table, space, CMD), None);
    }

    #[test]
    fn different_modifiers_are_a_different_combination() {
        let space = keycode::keycode_for("SPACE").unwrap();
        let table = [entry(0, space as i64, CMD as i64, true)];
        assert_eq!(held_by_system(&table, space, CMD | SHIFT), None);
        assert_eq!(held_by_system(&table, space, SHIFT), None);
    }

    #[test]
    fn bits_outside_the_four_are_ignored_on_both_sides() {
        let space = keycode::keycode_for("SPACE").unwrap();
        let noise = 1 << 15;
        let table = [entry(7, space as i64, CMD as i64 | noise, true)];
        assert_eq!(held_by_system(&table, space, CMD), Some(7));
    }

    #[test]
    fn an_empty_table_reports_nothing_held() {
        assert_eq!(held_by_system(&[], 49, CMD), None);
    }

    #[test]
    #[ignore = "reads the live system shortcut table"]
    fn the_live_table_can_be_read_and_holds_cmd_space() {
        let table = read_symbolic_hotkeys();
        assert!(
            !table.is_empty(),
            "CopySymbolicHotKeys returned nothing — the API or its dictionary keys moved"
        );
        let hk = hotkey::parse("Cmd+Space").unwrap();
        let code = keycode::keycode_for(hk.key.as_str()).unwrap();
        assert!(
            held_by_system(&table, code, keycode::carbon_modifiers(hk.modifiers)).is_some(),
            "⌘Space is not in the system table of {} entries — either Spotlight's \
             shortcut is disabled on this machine or the shape changed",
            table.len()
        );
    }
}
