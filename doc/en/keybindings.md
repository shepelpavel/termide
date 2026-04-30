# Keybindings

Termide canonicalizes every key event before matching it against a
binding. The canonicalization runs once at the dispatch boundary and
its result, together with the original raw event, travels through the
panel pipeline as a `KeyChord`. Panels and modals pick whichever form
they need:

- **`canonical`** — for hotkey matching, vim command interpretation,
  Settings keybinding capture.
- **`raw`** — for text input (`InsertChar`), terminal-panel PTY
  passthrough, search-buffer typing.

Text typed inside the editor or sent to a program running in the
terminal panel is **never** rewritten by canonicalization, so Cyrillic,
shifted glyphs, and locale-specific characters reach the destination
unchanged.

## What canonicalization fixes

| Quirk | Behaviour |
| --- | --- |
| Cyrillic letters on the same physical key as Latin (`й`/`q`, `ь`/`m`, …) | Mapped to Latin so a binding `Alt+M` fires whether the active layout is QWERTY or ЙЦУКЕН. |
| `REPORT_ALTERNATE_KEYS` shifted-glyph rewrite | Crossterm rewrites `Shift+Ctrl+=` → `Char('+') + Ctrl` (Shift stripped, char swapped). The normalizer reverses that — `Char('+') + Ctrl` → `Char('=') + Ctrl + Shift`. |
| Caps Lock spurious Shift on letters | When `REPORT_EVENT_TYPES` flagged `KeyEventState::CAPS_LOCK`, the Shift bit attached to letters is dropped before matching. |
| VTE `Ctrl+/` collapsing to `Ctrl+7` | Only when Kitty proto is **not** active: `Ctrl+7` → `Ctrl+/`. |

## Universal vs Enhanced bindings

Some chords cannot be encoded by every terminal. Termide groups
defaults into two tiers and warns at startup if the active terminal
cannot deliver any of the configured Enhanced-tier chords.

### Universal tier (works on every VT100+ terminal)

- `Alt+letter`, `Ctrl+letter` (letters → ASCII control 0x01–0x1A).
- `F1`–`F12` and `F1`–`F12` with **a single** modifier (`Shift+F*`,
  `Alt+F*`, `Ctrl+F*`).
- Arrows with **a single** modifier (`Shift+Up`, `Ctrl+Up`, `Alt+Up`).
- `Home`, `End`, `PgUp`, `PgDn` + a single modifier.
- `Enter`, `Tab`, `Esc`, `Backspace`, `Delete`, `Insert` + a single
  modifier.
- `Alt+digit`.
- `Alt+punctuation` (`Alt+/`, `Alt+,`, `Alt+.`, …).

### Enhanced tier (requires Kitty keyboard protocol)

- `Ctrl+punctuation` (`Ctrl+/`, `Ctrl+-`, `Ctrl+=`, `Ctrl+,`, `Ctrl+.`).
- `Ctrl+Shift+letter`.
- `Ctrl+Alt+anything`.
- `Alt+Shift+letter` and `Alt+Shift+arrow` — VTE in legacy mode emits
  `\eL` for `Alt+Shift+l`, indistinguishable from `Alt+L`; an
  `Alt+Shift+...` binding cannot match.
- `Super` / `Meta` / `Hyper` modifiers.

Enhanced-tier defaults that termide ships (`Ctrl+/` for `toggle_comment`
and `switch_directory`, `Ctrl+Alt+R` for `replace_all`) are kept because
they are de-facto standards across editors. On a terminal without
Kitty proto, termide logs a startup warning listing the affected
bindings; the user can rebind them through Settings → Keybindings.

## Terminal compatibility (2026)

| Terminal | Kitty keyboard protocol |
| --- | --- |
| kitty | full |
| foot 1.13+ | full |
| WezTerm | full |
| Ghostty | full |
| iTerm2 | full |
| rio | full |
| Windows Terminal Preview 1.25+ | full |
| alacritty | partial (CSI-u, no enhancement flags) |
| xterm | partial (manual config) |
| GNOME Terminal / Tilix / VTE | none (in progress) |
| Konsole | none (planned) |
| tmux | pass-through (depends on host terminal) |

If your terminal does not advertise Kitty proto and you rely on
Enhanced-tier chords, either switch to a supporting terminal or rebind
the affected actions to Universal-tier alternatives in
`config.toml` → `[*.keybindings]`.

## Conflict detection

Settings → Keybindings shows an inline warning when you assign a chord
already in use by another action. Three classes of conflict are
detected:

- **Same section** — two actions in the same section share the chord;
  the second one becomes unreachable.
- **Cross-section shadow** — a global chord shadows a panel-local one;
  the panel binding never fires.
- **Cross-section ambient** — two panel-local bindings overlap; only
  the focused panel handles the event, so usually fine but worth
  noting.

Same-section conflicts are also logged at startup.

## Customising defaults

Override any binding in `config.toml`. Strings are parsed in canonical
form, so `"Alt++"` ≡ `"Alt+Shift+="` and `"Ctrl+Й"` ≡ `"Ctrl+Q"`:

```toml
[general.keybindings]
panel_grow_vertical = "Alt+Shift+="
panel_shrink_vertical = "Alt+Shift+-"
open_sessions = "Alt+\\"

[editor.keybindings]
trigger_completion = ["Ctrl+J", "Ctrl+Space"]
toggle_comment = ["Ctrl+/", "Ctrl+."]
replace_all = ["Ctrl+Alt+R", "Alt+R"]

[file_manager.keybindings]
switch_directory = "Ctrl+\\"

[terminal.keybindings]
switch_directory = "Ctrl+\\"
```

Note: `Ctrl+/` and `Ctrl+\` work even on legacy terminals (e.g. VTE)
through `KeyNormalizer` quirks — VTE sends those as `\x1F` and `\x1C`
control bytes, which crossterm parses as `Ctrl+7` / `Ctrl+4` and the
normalizer rewrites back to the slash / backslash chord.

Multiple alternatives are supported for any action: list them in an
array. The first form is the canonical display string shown in help
panels.
