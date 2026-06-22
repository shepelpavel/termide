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
- **Inline find / replace bars** (`Ctrl+F` / `Ctrl+H`) - Docked search and replace inside the editor, file manager, and terminal, with live preview, a match counter, `[Aa]` / `[.*]` toggles, and navigation buttons
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

The menu bar is located at the top of the window and includes: menu items on the left, system resource indicators (network speed, CPU, RAM and — when a battery is present — its charge), and a clock in HH:MM format on the right.
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
  - [Operations](operations.md) — background operations panel
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

**System Resource Indicators** (clickable indicators open a details modal):

| Indicator | Description | Click opens |
|-----------|-------------|-------------|
| `↓…/↑…` | Network speed (↓ download, ↑ upload) | Top-10 processes by network activity |
| `CPU XX%` | CPU usage | Top-10 processes by CPU |
| `RAM X/YGB` | Memory usage | Top-10 processes by RAM |
| `⚡NN%` / `🔋NN%` | Battery charge (⚡ = AC connected / charging / full, 🔋 = on battery). Hidden on systems without a battery. | — (informational) |
| `DEVICE used/totalGB` (in status bar) | Disk usage | Top-10 processes by disk + filesystem partitions |

Clicking the same indicator again closes the window it opened (toggle); the same behaviour applies to the disk indicator in the status bar.

Color coding (CPU / RAM): green < 50%, yellow 50–75%, red > 75%. Battery uses the same scale inverted (green when charging or above 50%, red below 25%). The battery reading is cached for 5 seconds so the per-frame render path never touches `/sys`.

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
- [operations](operations.md)
- image viewer
- help — `Alt+H`
- journal — `Alt+L`

The **git log** panel draws the commit graph with box-drawing pseudographics
(`● │ ├ ╮ ╯`) laid out from each commit's parents, with each lane coloured so a
branch can be followed by colour. To fall back to git's native ASCII `--graph`
instead, set it in `config.toml`:

```toml
[git_log]
unicode_graph = false
```

**Features of closeable panels:**
- Have `[≡]` action button in panel title (click to open context menu with Close / Split / Merge / Move)
- Can be closed with Escape, Alt+X, or F10
- Column width adjustable with `Alt+=` / `Alt+-`
- Per-panel height adjustable inside a stacked column with `Alt+Shift+=` / `Alt+Shift+-` (1-row step), by dragging the panel's bottom border with the mouse, or by dragging the panel header up/down within the column
- `Alt+F11` toggles the "fullscreen current panel" preset (one panel fills the column, others collapse to one row); pressing it again restores the previous heights
- Can be dragged by the top border to another position (see Mouse Interaction below)

**Panel action context menu:**

Clicking the `[≡]` button on the panel header (or pressing `Alt+K` / `Shift+F10` on the active panel) opens a dropdown with:
- **Move up / Move down** — reorder inside the current group (when the group has more than one panel)
- **Move left / Move right** — move the panel to an adjacent group (when there is more than one group)
- **Split / Merge** — split the panel out of its group into a new one (when stacked), or merge a solo panel into its neighbour
- **Close** — close the panel (with confirmation when needed)

Items are filtered by context: e.g. *Split/Merge* is hidden when there is only one panel in one group, *Move up/down* is hidden for solo panels.

**Mouse Interaction:**
- Click on the title area activates the panel; double-click on a file-manager title opens the directory picker.
- **Drag a panel's bottom border** within a stacked column to resize its
  height (live `━` ghost line previews the new divider position, the
  resize is applied on release).
- **Drag a panel by its top border (title area)** to either resize or
  reorder the panel:
  - Release inside the source panel's body or the upper neighbour's
    body → vertical resize: the divider above the dragged panel snaps
    to the cursor (top panel of a group has no divider above and falls
    through to the move logic).
  - Release on another panel's header row in the same group → reorder:
    insert before / after the target depending on the source position.
  - Release over another panel's body in a different column → move the
    panel into that group, splitting the target panel at the drop row.
  - Release in the 2-cell gutter between two groups → create a new
    group at that position.
  - Release past the rightmost group → append as a new last group.
  - During the drag, a thick `━` line marks horizontal drop zones
    (resize / split inside a column) and a thick `┃` line marks
    vertical drop zones (new column between groups).
  - Press `Escape` during a drag to cancel.

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
| `Alt+Shift+=`     | Grow focused panel height (1 row)          |
| `Alt+Shift+-`     | Shrink focused panel height (1 row)        |
| `Alt+F11`         | Toggle fullscreen for the focused panel    |
| `Alt+Backspace`   | Toggle panel stacking (merge/unstack)      |
| `Alt+K`           | Open panel action menu (`[≡]` dropdown)    |
| `Shift+F10`       | Open panel action menu (alternative)       |
| `Alt+\`           | Open sessions menu                         |
| `Alt+N`           | Create new session                         |
| `Alt+B`           | Add bookmark                               |
| `Ctrl+P`          | Open command palette                       |
| `Alt+1-9`         | Jump to panel by number                    |

### Caps Lock

Termide opts into the [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) via `KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | REPORT_ALTERNATE_KEYS | REPORT_EVENT_TYPES`. `REPORT_EVENT_TYPES` exposes `KeyEventState::CAPS_LOCK`, which the hotkey matcher uses to ignore the spurious `Shift` modifier that X11/Linux terminals attach to every letter event while Caps Lock is on.

The practical effect:

- Bindings like `Alt+T` keep working with Caps Lock pressed (without this, the event would arrive as `{Char('T'), Alt|Shift}` and miss the `{Char('t'), Alt}` binding).
- Intentional `Shift+letter` bindings (e.g. `Ctrl+Shift+F` for content search) stay distinct from their unshifted counterparts — Shift is only ignored when Caps Lock is actually reported.
- Terminals without Kitty protocol support (Caps Lock bit not delivered) fall back to strict modifier comparison, so behaviour there is unchanged — use those terminals without Caps Lock for best results.

Modified arrow keys and Home/End are encoded for the embedded terminal emulator — see [terminal.md](terminal.md#interaction).
