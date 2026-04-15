//! Keyboard handling and layout translation.
//!
//! This crate provides utilities for keyboard event handling,
//! including translation between keyboard layouts (e.g., Cyrillic → Latin)
//! to ensure hotkeys work correctly regardless of active keyboard layout.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Cyrillic to Latin mapping table (ЙЦУКЕН → QWERTY)
///
/// Converts Cyrillic character to corresponding Latin character
/// on the same physical key.
pub fn cyrillic_to_latin(ch: char) -> char {
    match ch {
        // Top row lowercase: йцукенгшщзхъ → qwertyuiop[]
        'й' => 'q',
        'ц' => 'w',
        'у' => 'e',
        'к' => 'r',
        'е' => 't',
        'н' => 'y',
        'г' => 'u',
        'ш' => 'i',
        'щ' => 'o',
        'з' => 'p',
        'х' => '[',
        'ъ' => ']',

        // Top row uppercase: ЙЦУКЕНГШЩЗХЪ → QWERTYUIOP[]
        'Й' => 'Q',
        'Ц' => 'W',
        'У' => 'E',
        'К' => 'R',
        'Е' => 'T',
        'Н' => 'Y',
        'Г' => 'U',
        'Ш' => 'I',
        'Щ' => 'O',
        'З' => 'P',
        'Х' => '{',
        'Ъ' => '}',

        // Middle row lowercase: фывапролджэ → asdfghjkl;'
        'ф' => 'a',
        'ы' => 's',
        'в' => 'd',
        'а' => 'f',
        'п' => 'g',
        'р' => 'h',
        'о' => 'j',
        'л' => 'k',
        'д' => 'l',
        'ж' => ';',
        'э' => '\'',

        // Middle row uppercase: ФЫВАПРОЛДЖЭ → ASDFGHJKL:"
        'Ф' => 'A',
        'Ы' => 'S',
        'В' => 'D',
        'А' => 'F',
        'П' => 'G',
        'Р' => 'H',
        'О' => 'J',
        'Л' => 'K',
        'Д' => 'L',
        'Ж' => ':',
        'Э' => '"',

        // Bottom row lowercase: ячсмитьбю → zxcvbnm,.
        'я' => 'z',
        'ч' => 'x',
        'с' => 'c',
        'м' => 'v',
        'и' => 'b',
        'т' => 'n',
        'ь' => 'm',
        'б' => ',',
        'ю' => '.',

        // Bottom row uppercase: ЯЧСМИТЬБЮ → ZXCVBNM<>
        'Я' => 'Z',
        'Ч' => 'X',
        'С' => 'C',
        'М' => 'V',
        'И' => 'B',
        'Т' => 'N',
        'Ь' => 'M',
        'Б' => '<',
        'Ю' => '>',

        // Punctuation keys that differ between ЙЦУКЕН and QWERTY
        // (physical key position mapping, not character translation)
        '.' => '/', // Точка (рус) → Slash (eng) — key next to right Shift
        ',' => '.', // Запятая (рус) → Period (eng)
        'ё' => '`', // Ё → backtick (top-left key)
        'Ё' => '~', // Ё + Shift → tilde

        // No change for other characters
        _ => ch,
    }
}

/// Translate KeyEvent for hotkeys
///
/// Applies Cyrillic → Latin translation only when modifier
/// (Ctrl or Alt) is pressed, to not affect regular text input.
pub fn translate_hotkey(key: KeyEvent) -> KeyEvent {
    // Normalize Ctrl+/ — legacy terminals send it as 0x1F (Unit Separator)
    if key.code == KeyCode::Char('\x1f') {
        return KeyEvent::new(KeyCode::Char('/'), key.modifiers | KeyModifiers::CONTROL);
    }

    // Apply only if modifier is present (Ctrl or Alt)
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        if let KeyCode::Char(ch) = key.code {
            let translated = cyrillic_to_latin(ch);
            if translated != ch {
                // Create new KeyEvent with translated character
                return KeyEvent::new(KeyCode::Char(translated), key.modifiers);
            }
        }
    }
    key
}

/// Translate Cyrillic characters to Latin regardless of modifiers.
///
/// Use in panel hotkey handlers where all input is treated as commands,
/// not as text input. This ensures hotkeys like `o`, `d`, `r`, `v` etc.
/// work correctly when the user's keyboard layout is set to Cyrillic.
pub fn translate_all_chars(key: KeyEvent) -> KeyEvent {
    if let KeyCode::Char(ch) = key.code {
        let translated = cyrillic_to_latin(ch);
        if translated != ch {
            return KeyEvent::new(KeyCode::Char(translated), key.modifiers);
        }
    }
    key
}

/// Normalize key for hotkey matching: Cyrillic → Latin QWERTY.
///
/// Applied to ALL character keys regardless of modifiers.
/// Does NOT replace the original KeyEvent — used as an **alternative**
/// for matching. HotkeyTable.matches() checks both raw and normalized.
///
/// Also fixes legacy Ctrl+/ (0x1F) terminal encoding.
pub fn normalize_for_matching(key: &KeyEvent) -> KeyEvent {
    // Normalize Ctrl+/ — legacy terminals send it as 0x1F (Unit Separator)
    if key.code == KeyCode::Char('\x1f') {
        return KeyEvent::new(KeyCode::Char('/'), key.modifiers | KeyModifiers::CONTROL);
    }
    // Normalize Ctrl+7 → Ctrl+/ — VTE terminals (GNOME Terminal) send Ctrl+/ as Ctrl+7
    if key.code == KeyCode::Char('7') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return KeyEvent::new(KeyCode::Char('/'), key.modifiers);
    }
    // Cyrillic → Latin for any char key (with or without modifiers)
    if let KeyCode::Char(ch) = key.code {
        let translated = cyrillic_to_latin(ch);
        if translated != ch {
            return KeyEvent::new(KeyCode::Char(translated), key.modifiers);
        }
    }
    *key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cyrillic_to_latin() {
        // Lowercase
        assert_eq!(cyrillic_to_latin('й'), 'q');
        assert_eq!(cyrillic_to_latin('ф'), 'a');
        assert_eq!(cyrillic_to_latin('я'), 'z');
        // Uppercase preserves case
        assert_eq!(cyrillic_to_latin('Й'), 'Q');
        assert_eq!(cyrillic_to_latin('Ф'), 'A');
        assert_eq!(cyrillic_to_latin('Я'), 'Z');
        // Non-Cyrillic unchanged
        assert_eq!(cyrillic_to_latin('q'), 'q'); // Latin unchanged
        assert_eq!(cyrillic_to_latin('1'), '1'); // Numbers unchanged
    }

    #[test]
    fn test_translate_hotkey_with_alt() {
        let key = KeyEvent::new(KeyCode::Char('й'), KeyModifiers::ALT);
        let translated = translate_hotkey(key);
        assert_eq!(translated.code, KeyCode::Char('q'));
        assert_eq!(translated.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn test_translate_hotkey_with_ctrl() {
        let key = KeyEvent::new(KeyCode::Char('ы'), KeyModifiers::CONTROL);
        let translated = translate_hotkey(key);
        assert_eq!(translated.code, KeyCode::Char('s'));
        assert_eq!(translated.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn test_no_translate_without_modifier() {
        let key = KeyEvent::new(KeyCode::Char('й'), KeyModifiers::NONE);
        let translated = translate_hotkey(key);
        assert_eq!(translated.code, KeyCode::Char('й')); // Unchanged
    }

    #[test]
    fn test_no_translate_shift_only() {
        let key = KeyEvent::new(KeyCode::Char('Й'), KeyModifiers::SHIFT);
        let translated = translate_hotkey(key);
        assert_eq!(translated.code, KeyCode::Char('Й')); // Unchanged
    }

    #[test]
    fn test_translate_ctrl_slash() {
        // Legacy terminal: sends 0x1F without modifiers
        let key = KeyEvent::new(KeyCode::Char('\x1f'), KeyModifiers::NONE);
        let translated = translate_hotkey(key);
        assert_eq!(translated.code, KeyCode::Char('/'));
        assert_eq!(translated.modifiers, KeyModifiers::CONTROL);

        // Some terminals: sends 0x1F with CONTROL
        let key = KeyEvent::new(KeyCode::Char('\x1f'), KeyModifiers::CONTROL);
        let translated = translate_hotkey(key);
        assert_eq!(translated.code, KeyCode::Char('/'));
        assert_eq!(translated.modifiers, KeyModifiers::CONTROL);
    }
}
