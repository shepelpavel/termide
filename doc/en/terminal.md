# Terminal

The terminal panel provides a full-featured terminal emulator with pseudoterminal (PTY) support, ensuring compatibility with most console applications such as `bash`, `fish`, `htop`, and `mc`.

## Key Features

- **Interactive Shell**: Launches the default system shell (`fish`, `zsh`, `bash`, etc.) for command execution
- **Compatibility**: Supports `xterm-256color` and most standard ANSI control sequences, ensuring correct display of colors and text styles
- **Process Management**: When closing a terminal panel with running processes, the application will request confirmation before terminating them

## Interaction

| Shortcut               | Action                                     |
|------------------------|--------------------------------------------|
| `Ctrl+/`               | Open directory switcher                    |
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

**Modified arrow keys and Home/End** are encoded as the standard xterm CSI
`1;{mod}{final}` escape sequence (`{mod}` is the xterm modifier parameter:
`2` = Shift, `3` = Alt, `5` = Ctrl, `6` = Ctrl+Shift, and so on; `{final}`
is `A`/`B`/`C`/`D`/`H`/`F`). In practice this means `Ctrl+Left` / `Ctrl+Right`
trigger `backward-word` / `forward-word` in bash/zsh readline, `Shift+Home` /
`Shift+End` select to line boundaries where the shell supports it, and so on.
Plain arrows keep their existing path, including application-cursor-mode
substitution (`\x1bOA` vs `\x1b[A`). `Alt+Left` / `Alt+Right` remain bound
globally to previous/next panel group and therefore aren't forwarded.

## Text Search

Press `Ctrl+F` to open the search modal. The search works across the entire scrollback buffer and the visible screen:

- **Live preview**: Matches are highlighted as you type
- **Match counter**: Shows current match position (e.g., "3 of 12")
- **Navigation**: `Tab` / `Shift+Tab` to jump between matches
- **Scroll**: The viewport automatically scrolls to the current match
- **Close**: `Escape` to close search and return to normal mode

The search keybinding defaults to `Ctrl+F` (instead of `Ctrl+Shift+F`) because most host terminals intercept `Ctrl+Shift+F` for their own search.

## Shell Selection

You can choose which shell to launch via the **Windows > Terminal** submenu. The submenu lists all shells detected on your system:

- **Linux/macOS**: shells from `/etc/shells`, plus common paths (`/usr/bin/fish`, `/usr/bin/zsh`, `/bin/bash`, `/bin/sh`) and NixOS-specific paths
- **Windows**: Git Bash, PowerShell Core (`pwsh`), Windows PowerShell, Command Prompt (`cmd`), and WSL distributions

The currently configured default shell is marked with **●**. Selecting a shell opens a new terminal with that shell and saves it as the default for future terminals.

You can also set the default shell in `config.toml`:

```toml
[terminal]
default_shell = "/usr/bin/fish"
```

## Mouse Support

- **Text Selection**: Click and hold the left mouse button to select text. Selected text is automatically copied to the clipboard after releasing the button
- **Scroll Wheel**: Scroll through terminal output history
- **Ctrl+Click on URL/path**: Open link in browser or file manager
- **Ctrl+Click on hex color**: Show color preview popup (e.g. `#ff0000`, `#abc`) — visible while button is held, disappears on release
- **Application Interaction**: If a console application (e.g., `htop` or `mc`) supports mouse input, the terminal will pass mouse events to it
