# Application Window Overview

**The application occupies all available space and has vertical division into elements:**
- Menu bar
- Panels area
- Status bar

The application also uses popup windows:
- Help
- Settings
- Application close confirmation

## Modal Windows

The application uses interactive modal windows for various operations:
- **Search Modal** (`Ctrl+F`) - Interactive search with live preview, match counter, and navigation buttons
- **Replace Modal** (`Ctrl+H`) - Interactive replace with two input fields, live search, and action buttons
- **Input Modals** - Various prompts for file operations (create, rename, etc.)
- **Confirmation Dialogs** - Delete confirmations, unsaved changes, etc.

**Modal Features:**
- `[X]` close button in modal title bar (clickable with mouse)
- Keyboard navigation with Tab/Shift+Tab (between modal fields / buttons)
- Mouse support for all buttons
- Escape key to close modal
- Live preview for search/replace operations
- State preservation (last entered text saved)

### Settings Modal

Opened with `Alt+P` or menu **Options → Edit preferences**. The previous horizontal-tab layout has been replaced by a **sidebar layout**:

**Three zones:**
- **Left sidebar** (18 columns wide) — section list, navigated with Up/Down
- **Content area** — fields of the selected section, organised under `── Group ──` subheaders and separated by blank spacers
- **Bottom buttons** — `Apply & Save`, `Reset`, `Cancel`

**Sidebar sections:**
- General, Editor, File Manager, Terminal, LSP, Logging, VFS — plain leaves
- **Keybindings** — expandable group (▶/▼) with 7 sub-sections (Global, Editor, FileManager, GitStatus, GitDiff, GitLog, Terminal)

**Navigation:**

| Shortcut | Action |
|----------|--------|
| `Up` / `Down` | Move within the current zone (sections / fields / buttons) |
| `Tab` / `Shift+Tab` | Cycle focus zones (Sidebar ↔ Content ↔ Buttons) |
| `Left` / `Right` | Do **not** change the zone: used for editing values (cycle enum, toggle bool) and for collapsing the Keybindings group in the sidebar |
| `Enter` / `Space` | Activate: in sidebar — enter the section; in content — toggle bool/enum or start editing number/text; on a group header — toggle expand |
| `Escape` | Close the modal (Cancel) |
| Mouse wheel | Scroll sidebar / content |

**Fields and indicators:**
- **Bool** — `[✓]` (on) or `[✗]` (off), toggled with `Enter`/`Space` or `Left`/`Right`
- **Enum** — `< value >`, cycled with `Left`/`Right`
- **Number** / **OptionalText** — `Enter` enters inline edit mode
- **LSP → Servers** — server list items are prefixed with a bullet `•`; there is also a `+ Add Server` row

**Keybindings — key capture:**  
Navigate bindings with `Up/Down`, press `Enter` to enter Capturing mode (the next keypress becomes the new binding), `Delete`/`Backspace` clears a binding.

The active section highlight in the sidebar is cleared when focus leaves it — the current section name is shown in the content-area header, so a dual highlight would just be misleading.

## Menu Bar

The menu bar is located at the top of the window and includes: menu items on the left, system resource indicators (CPU, RAM, network speed), and a clock in HH:MM format on the right.
Menu activation/deactivation and each item can be accessed by mouse click or [keyboard shortcuts](#Keyboard-Navigation-and-Panel-Management).

**Menu items:**
- `Sessions` — session management submenu:
  - New session — create session in a new directory
  - Switch session — open session switcher modal
  - Change root path — move current session to another directory
- `Windows` — panel creation submenu:
  - Files — file manager panel
  - Terminal — terminal panel (has submenu for choosing a shell: lists all available shells on the system, the default shell is marked with ●)
  - Editor — text editor panel
  - Git Status — git status panel
  - Git Log — commit history panel
  - Git Stash — git stash management panel
  - Journal — application log panel
  - Diagnostics — LSP diagnostics panel
  - Operations — background operations panel
  - Outline — structural code navigation panel
- `Scripts` — user-defined scripts (with group submenus). Clicking a group header expands the submenu; clicking the same header again collapses it (toggle).
- `Bookmarks` — saved locations (directories, files, SSH, SFTP). Group behaviour is the same toggle as in Scripts.
- `Options` — settings submenu:
  - Themes — theme selection with live preview
  - Language — UI language with live preview
  - Manage scripts — open scripts folder
  - Manage bookmarks — open bookmarks file
  - Edit preferences — open config.toml in editor
  - Help — open help panel
  - Quit — exit application

**System Resource Indicators** (all clickable — open a details modal):

| Indicator | Description | Click opens |
|-----------|-------------|-------------|
| `CPU XX%` | CPU usage | Top-10 processes by CPU |
| `RAM X/YGB` | Memory usage | Top-10 processes by RAM |
| `↓…/↑…` | Network speed (↓ download, ↑ upload) | Top-10 processes by network activity |
| `DEVICE used/totalGB` (in status bar) | Disk usage | Top-10 processes by disk + filesystem partitions |

Clicking the same indicator again closes the window it opened (toggle); the same behaviour applies to the disk indicator in the status bar.

Color coding: green < 50%, yellow 50–75%, red > 75%.

## Panels Area

The area fills the vertical space between the menu bar and status bar from left to right edge of the window.
The layout adapts to the terminal width, showing more panel groups on wider screens. Panels (terminal, editor, git) can be opened and will be placed alongside.

**Possible openable panel types:**
- [file manager](file-manager.md) — `Alt+F`
- [terminal](terminal.md) — `Alt+T`
- [text editor](editor.md) — `Alt+E`
- git status — `Alt+G`
- outline — `Alt+O`
- diagnostics — `Alt+I`
- git log — `Alt+C`
- git diff
- operations
- image viewer
- help — `Alt+H`
- journal — `Alt+L`

**Features of closeable panels:**
- Have `[X]` close button in panel title (clickable with mouse)
- Can be closed with Escape, Alt+X, or Alt+Backspace
- Can be resized with Alt+Plus/Minus

## Status Bar

The status bar is designed to display additional information about work in the active panel.
Depending on the type of active panel, corresponding data is displayed.

### Disk Space Indicator

The status bar shows disk space information on the right side in the format: `DEVICE used/totalGB (usage%)` with color coding based on usage level:

**Color Coding:**
- **Green** when disk usage < 50%
- **Yellow** when disk usage 50-75%
- **Red** when disk usage > 75%

**Format:** `DEVICE used/total (usage%)`

Example: `NVME0N1P2 386/467Gb (83%)`

The device name is automatically detected from the filesystem:
- On Linux: shows partition names like `NVME0N1P2`, `SDA1`, etc.
- On macOS: shows disk identifiers
- The displayed device corresponds to the partition where the current directory is located

## Keyboard Navigation and Panel Management

| Shortcut          | Action                                     |
|-------------------|--------------------------------------------|
| `Alt+M`           | Activate / deactivate menu                 |
| `Alt+F`           | Open file manager panel                    |
| `Alt+T`           | Open terminal panel                        |
| `Alt+E`           | Open new file editor panel                 |
| `Alt+G`           | Open git status panel                      |
| `Alt+O`           | Open outline panel                         |
| `Alt+I`           | Open diagnostics panel                     |
| `Alt+C`           | Open git log panel                         |
| `Alt+L`           | Open journal panel                             |
| `Alt+P`           | Open configuration file in editor          |
| `Alt+H`           | Open help window                           |
| `Alt+Q`           | Close application                          |
| `Escape`          | Close panel / Close modal                  |
| `Alt+X`           | Close panel                                |
| `Alt+Delete`      | Close panel                                |
| `Alt+Left`        | Go to previous panel group (horizontal)    |
| `Alt+Right`       | Go to next panel group (horizontal)        |
| `Alt+Up`          | Go to previous panel in group (vertical)   |
| `Alt+Down`        | Go to next panel in group (vertical)       |
| `Alt+W/S/A/D`     | WASD-style panel navigation (alternative to arrows) |
| `Alt+PgUp`        | Move panel to previous group               |
| `Alt+PgDn`        | Move panel to next group                   |
| `Alt+Home`        | Move panel to first group                  |
| `Alt+End`         | Move panel to last group                   |
| `Alt+Plus (=)`    | Increase active group width                |
| `Alt+Minus (-)`   | Decrease active group width                |
| `Alt+Backspace`   | Toggle panel stacking (merge/unstack)      |
| `Alt+/`           | Open sessions menu                         |
| `Alt+N`           | Create new session                         |
| `Alt+K`           | Add bookmark                               |
| `Ctrl+P`          | Open command palette                       |
| `Ctrl+Shift+P`    | Open command palette (alternative)         |
| `Alt+1-9`         | Jump to panel by number                    |
| `Ctrl+Alt+1-9`    | Jump to panel by number (fallback for gnome-terminal / Windows Terminal) |
