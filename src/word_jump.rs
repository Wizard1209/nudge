//! Unicode-aware word boundary helpers for TextEdit Ctrl+Arrow / Ctrl+Backspace
//! / Ctrl+Delete. Egui 0.31's built-in `is_word_char` is ASCII-only, so Cyrillic
//! (and other non-ASCII) text breaks word jumping. We override that with an
//! `is_alphanumeric()`-based predicate. Indices are *char* indices (matching
//! egui's `CCursor::index`), not byte indices.

use eframe::egui;

/// Returns the char index of the next word boundary at or after `cursor`.
/// Mirrors egui's `next_word_boundary_char_index` semantics but Unicode-aware.
pub fn next_word_boundary(text: &str, cursor: usize) -> usize {
    next_boundary_in_iter(text.chars(), cursor)
}

/// Returns the char index of the previous word boundary at or before `cursor`.
pub fn previous_word_boundary(text: &str, cursor: usize) -> usize {
    let num_chars = text.chars().count();
    num_chars - next_boundary_in_iter(text.chars().rev(), num_chars - cursor)
}

fn next_boundary_in_iter(it: impl Iterator<Item = char>, mut index: usize) -> usize {
    let mut it = it.skip(index);
    if it.next().is_some() {
        index += 1;
        if let Some(second) = it.next() {
            index += 1;
            let second_is_word = is_word_char(second);
            for next in it {
                if is_word_char(next) != second_is_word {
                    break;
                }
                index += 1;
            }
        }
    }
    index
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn char_idx_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn delete_char_range(s: &mut String, start_char: usize, end_char: usize) {
    let start_byte = char_idx_to_byte(s, start_char);
    let end_byte = char_idx_to_byte(s, end_char);
    s.replace_range(start_byte..end_byte, "");
}

/// Intercept Ctrl+Arrow / Ctrl+Backspace / Ctrl+Delete on a focused TextEdit
/// and replace egui's ASCII-only word-jump with our Unicode-aware version.
///
/// Call BEFORE drawing the TextEdit. Removes handled events from the input
/// queue so TextEdit does not double-process them. Also mirrors the action on
/// `Alt` to match egui's mac binding parity.
pub fn intercept_ctrl_word_keys(
    ctx: &egui::Context,
    field_id: egui::Id,
    value: &mut String,
) {
    use egui::text::{CCursor, CCursorRange};
    use egui::{Event, Key};

    // Cheap pre-check: any relevant event in the queue?
    let has_relevant = ctx.input(|i| {
        i.events.iter().any(|e| {
            matches!(
                e,
                Event::Key {
                    key: Key::ArrowLeft | Key::ArrowRight | Key::Backspace | Key::Delete,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.ctrl || modifiers.alt
            )
        })
    });
    if !has_relevant {
        return;
    }

    let Some(mut state) =
        egui::widgets::text_edit::TextEditState::load(ctx, field_id)
    else {
        return;
    };
    let mut range = state.cursor.char_range().unwrap_or_else(|| {
        let end = value.chars().count();
        CCursorRange::one(CCursor::new(end))
    });

    let mut changed = false;

    ctx.input_mut(|i| {
        i.events.retain(|event| {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            if !(modifiers.ctrl || modifiers.alt) {
                return true;
            }
            let [min, max] = range.sorted();
            let has_selection = min.index != max.index;
            match key {
                Key::ArrowLeft => {
                    let target = previous_word_boundary(value, range.primary.index);
                    range.primary = CCursor::new(target);
                    if !modifiers.shift {
                        range.secondary = range.primary;
                    }
                    changed = true;
                    false
                }
                Key::ArrowRight => {
                    let target = next_word_boundary(value, range.primary.index);
                    range.primary = CCursor::new(target);
                    if !modifiers.shift {
                        range.secondary = range.primary;
                    }
                    changed = true;
                    false
                }
                Key::Backspace => {
                    let new_index = if has_selection {
                        delete_char_range(value, min.index, max.index);
                        min.index
                    } else {
                        let target = previous_word_boundary(value, min.index);
                        delete_char_range(value, target, min.index);
                        target
                    };
                    range = CCursorRange::one(CCursor::new(new_index));
                    changed = true;
                    false
                }
                Key::Delete => {
                    let new_index = if has_selection {
                        delete_char_range(value, min.index, max.index);
                        min.index
                    } else {
                        let target = next_word_boundary(value, min.index);
                        delete_char_range(value, min.index, target);
                        min.index
                    };
                    range = CCursorRange::one(CCursor::new(new_index));
                    changed = true;
                    false
                }
                _ => true,
            }
        });
    });

    if changed {
        state.cursor.set_char_range(Some(range));
        state.store(ctx, field_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- next_word_boundary ----

    #[test]
    fn next_ascii_skips_word_then_stops_before_space() {
        // "hello world" — from start, jump after "hello", leave the space
        assert_eq!(next_word_boundary("hello world", 0), 5);
    }

    #[test]
    fn next_ascii_from_space_consumes_space_and_word() {
        // "hello world" — from the space, consume " world" to end
        assert_eq!(next_word_boundary("hello world", 5), 11);
    }

    #[test]
    fn next_at_end_is_noop() {
        assert_eq!(next_word_boundary("hello", 5), 5);
    }

    #[test]
    fn next_empty_is_noop() {
        assert_eq!(next_word_boundary("", 0), 0);
    }

    #[test]
    fn next_single_char_advances_to_end() {
        assert_eq!(next_word_boundary("a", 0), 1);
    }

    #[test]
    fn next_cyrillic_stops_before_space() {
        // "привет мир" — 6 char "привет", space, 3 char "мир"
        assert_eq!(next_word_boundary("привет мир", 0), 6);
    }

    #[test]
    fn next_cyrillic_from_space_consumes_to_end() {
        assert_eq!(next_word_boundary("привет мир", 6), 10);
    }

    #[test]
    fn next_mixed_stops_at_script_change_only_via_space() {
        // "test тест" — "test" is word, space is not, "тест" is word.
        // From 0: stop after "test" (before space).
        assert_eq!(next_word_boundary("test тест", 0), 4);
        // From space: consume space + "тест" to end (9 chars total).
        assert_eq!(next_word_boundary("test тест", 4), 9);
    }

    // ---- previous_word_boundary ----

    #[test]
    fn prev_ascii_jumps_to_word_start() {
        // "hello world" — from end, back to start of "world"
        assert_eq!(previous_word_boundary("hello world", 11), 6);
    }

    #[test]
    fn prev_ascii_through_space_to_start() {
        assert_eq!(previous_word_boundary("hello world", 6), 0);
    }

    #[test]
    fn prev_at_start_is_noop() {
        assert_eq!(previous_word_boundary("hello", 0), 0);
    }

    #[test]
    fn prev_empty_is_noop() {
        assert_eq!(previous_word_boundary("", 0), 0);
    }

    #[test]
    fn prev_cyrillic_jumps_to_word_start() {
        // "привет мир" — from end, back to start of "мир"
        assert_eq!(previous_word_boundary("привет мир", 10), 7);
    }

    #[test]
    fn prev_cyrillic_through_space_to_start() {
        assert_eq!(previous_word_boundary("привет мир", 7), 0);
    }

    #[test]
    fn prev_mixed() {
        // "test тест" — from end (9), back to start of "тест" (5)
        assert_eq!(previous_word_boundary("test тест", 9), 5);
        // From 5 (start of "тест"), back to start of "test" (0)
        assert_eq!(previous_word_boundary("test тест", 5), 0);
    }
}
