//! Pair of `(raw, canonical)` views of a single physical keypress.
//!
//! Built once at the dispatch boundary (`crates/app/src/app/key_handler.rs`)
//! and propagated through the entire pipeline. Consumers explicitly
//! pick the form they need:
//!
//! - **`canonical`**: post-`KeyNormalizer` form for hotkey matching
//!   (`HotkeyTable::matches`, vim command interpretation, settings
//!   keybinding capture).
//! - **`raw`**: exactly what crossterm reported; for text input
//!   (`InsertChar`), PTY passthrough (`modern_key_bytes`),
//!   search-buffer typing.
//!
//! Sticking to one direction (raw → canonical, never the reverse)
//! keeps the type cheap (Copy, two `KeyEvent`s) and makes the
//! passthrough invariant trivially testable: the `raw` field equals
//! the original crossterm event byte-for-byte.

use crossterm::event::KeyEvent;
use termide_keyboard::KeyNormalizer;

/// Pair of (raw, canonical) views of the same physical keypress.
///
/// See module docs for which form to use where.
#[derive(Debug, Clone, Copy)]
pub struct KeyChord {
    pub raw: KeyEvent,
    pub canonical: KeyEvent,
}

impl KeyChord {
    /// Constructs a `KeyChord` from the raw crossterm event by running
    /// `KeyNormalizer::canonicalize`.
    pub fn new(raw: KeyEvent, normalizer: &KeyNormalizer) -> Self {
        let canonical = normalizer.canonicalize(raw);
        Self { raw, canonical }
    }

    /// Constructs a `KeyChord` where both forms are identical. Useful in
    /// tests and for code paths that don't have a normalizer available
    /// (e.g. simple key constants in tests).
    pub fn identity(key: KeyEvent) -> Self {
        Self {
            raw: key,
            canonical: key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState, KeyModifiers};
    use termide_keyboard::KeyboardCaps;

    fn make(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn raw_unchanged_after_canonicalization() {
        let n = KeyNormalizer::new(KeyboardCaps::default());
        let raw = make(KeyCode::Char('й'), KeyModifiers::ALT);
        let chord = KeyChord::new(raw, &n);
        assert_eq!(chord.raw, raw);
        assert_eq!(chord.canonical.code, KeyCode::Char('q'));
    }

    #[test]
    fn identity_chord() {
        let raw = make(KeyCode::Char('a'), KeyModifiers::CONTROL);
        let chord = KeyChord::identity(raw);
        assert_eq!(chord.raw, chord.canonical);
        assert_eq!(chord.raw, raw);
    }
}
