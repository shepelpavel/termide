# Terminal

The terminal panel provides a full-featured terminal emulator with pseudoterminal (PTY) support, ensuring compatibility with most console applications such as `bash`, `fish`, `htop`, and `mc`.

## Key Features

- **Interactive Shell**: Launches the default system shell (`fish`, `zsh`, `bash`, etc.) for command execution
- **Compatibility**: Supports `xterm-256color` and most standard ANSI control sequences, ensuring correct display of colors and text styles
- **Process Management**: When closing a terminal panel with running processes, the application will request confirmation before terminating them

## Interaction

| Shortcut               | Action                                     |
|------------------------|--------------------------------------------|
| `Ctrl+F`               | Open text search in scrollback             |
| `Ctrl+Shift+C`         | Copy selected text to clipboard            |
| `Ctrl+Shift+V`         | Paste text from clipboard                  |
| `Ctrl+Shift+М`         | Paste text from clipboard (Cyrillic layout)|
| `Shift+Enter`          | Insert newline (multi-line input)          |
| `Shift+PageUp`         | Scroll output history up                   |
| `Shift+PageDown`       | Scroll output history down                 |
| `Shift+Home`           | Go to beginning of output history          |
| `Shift+End`            | Go to current line (end of history)        |

**Keyboard Layout Support:**

TermIDE supports Cyrillic keyboard layouts for common shortcuts. When using a Russian/Cyrillic layout, you can use `Ctrl+Shift+М` (where М is the Cyrillic letter corresponding to V) instead of switching to Latin layout. This works for paste operations in the terminal.

All other key combinations are passed directly to the application running in the terminal.

## Text Search

Press `Ctrl+F` to open the search modal. The search works across the entire scrollback buffer and the visible screen:

- **Live preview**: Matches are highlighted as you type
- **Match counter**: Shows current match position (e.g., "3 of 12")
- **Navigation**: `Tab` / `Shift+Tab` to jump between matches
- **Scroll**: The viewport automatically scrolls to the current match
- **Close**: `Escape` to close search and return to normal mode

The search keybinding defaults to `Ctrl+F` (instead of `Ctrl+Shift+F`) because most host terminals intercept `Ctrl+Shift+F` for their own search.

## Mouse Support

- **Text Selection**: Click and hold the left mouse button to select text. Selected text is automatically copied to the clipboard after releasing the button
- **Scroll Wheel**: Scroll through terminal output history
- **Application Interaction**: If a console application (e.g., `htop` or `mc`) supports mouse input, the terminal will pass mouse events to it
