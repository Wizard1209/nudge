//! The system's own shortcut table, read through `CopySymbolicHotKeys`.
//!
//! This is the source that catches `⌘Space`, `⌘Tab` and the screenshot
//! shortcuts — the combinations the registration call accepts without a word
//! and which then never fire, because macOS gets the keystroke first. The
//! exclusive-registration probe cannot see them (they are not exclusive Carbon
//! registrations at all), and this cannot see another application's binding,
//! which is why conflict detection needs both and says which one answered.
//!
//! The read and the decision are split on purpose: [`read_symbolic_hotkeys`]
//! touches the OS, [`held_by_system`] is a pure function over what it returned,
//! so the comparison is table-testable without a Mac in a particular state.
//!
//! Like the Carbon Event Manager next door, this API has no header in the
//! macOS 26 SDK — `CopySymbolicHotKeys` is exported by `HIToolbox` and its
//! three dictionary keys are not exported at all, so they are spelled out as
//! literal `CFString`s below. A shape change would show up as an empty table
//! rather than as a wrong answer: every entry that does not parse is skipped
//! and counted, and the count is logged.

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

/// The keys of one entry. Not exported as symbols by the framework, so the
/// names are the strings themselves.
const KEY_CODE: &str = "kHISymbolicHotKeyCode";
const KEY_MODIFIERS: &str = "kHISymbolicHotKeyModifiers";
const KEY_ENABLED: &str = "kHISymbolicHotKeyEnabled";

/// One row of the system's shortcut table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolicHotkey {
    /// Which row this was. macOS does not put the symbolic hot key's id in the
    /// dictionary, so this is the position in the array — enough to say *which*
    /// system shortcut collided when the same combination appears twice, and
    /// honest about being no more than that.
    pub row: i64,
    /// The virtual key code, the same integer this app registers with.
    pub code: i64,
    /// Carbon modifier bits (`cmdKey`, `shiftKey`, `optionKey`, `controlKey`).
    pub carbon_modifiers: i64,
    /// A disabled entry holds nothing — the user turned that shortcut off, and
    /// the combination is free.
    pub enabled: bool,
}

/// The four bits a Nudge hotkey can carry. The system table sometimes sets
/// others (the right-hand modifier bits, for instance), and comparing the raw
/// masks would then miss a real collision.
const CARBON_MODIFIER_MASK: i64 = (1 << 8) | (1 << 9) | (1 << 11) | (1 << 12);

/// Read the table. An empty result means "we could not see it", never "nothing
/// is bound" — every caller treats it as the best-effort answer it is.
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
    // Create rule: the array is ours to release, and the wrapper does it.
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
            // A missing flag is read as enabled: the conservative answer is
            // "somebody may hold this", which produces a warning rather than a
            // silent acceptance.
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
    // Type-checked rather than assumed: reading a CFBoolean as a CFNumber
    // would produce a number, and a wrong one.
    if unsafe { CFGetTypeID(value) } != CFNumber::type_id() {
        return None;
    }
    let number = unsafe { CFNumber::wrap_under_get_rule(value as _) };
    number.to_i64()
}

fn dict_bool(dict: CFDictionaryRef, key: &CFString) -> Option<bool> {
    let value = dict_value(dict, key)?;
    if unsafe { CFGetTypeID(value) } != CFBoolean::type_id() {
        // Some entries carry the flag as a number rather than a boolean.
        return dict_i64(dict, key).map(|n| n != 0);
    }
    Some(unsafe { CFBoolean::wrap_under_get_rule(value as _) }.into())
}

/// Whether an enabled system shortcut holds this combination, and which row it
/// was. Pure — the whole decision, over a slice the caller supplies.
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
        // The user turned that system shortcut off, so the combination is
        // free and warning about it would be noise.
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
        // The table carries side-specific and legacy bits; comparing raw masks
        // would miss a real collision.
        let space = keycode::keycode_for("SPACE").unwrap();
        let noise = 1 << 15;
        let table = [entry(7, space as i64, CMD as i64 | noise, true)];
        assert_eq!(held_by_system(&table, space, CMD), Some(7));
    }

    #[test]
    fn an_empty_table_reports_nothing_held() {
        // What a failed read produces. It must read as "we cannot see a
        // holder", which is what `Free` means everywhere in this module.
        assert_eq!(held_by_system(&[], 49, CMD), None);
    }

    /// The instrument itself, against the live system. `#[ignore]` because it
    /// reads the machine's own settings and its answer depends on them — but
    /// when it runs, it is the only check that the dictionary keys above still
    /// name anything.
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
