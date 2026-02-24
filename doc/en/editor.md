# Text Editor

The text editor panel provides a functional editor for working with text files with syntax highlighting support for various programming languages.

## Key Features

- **Syntax Highlighting**: Automatic highlighting for popular programming languages (Rust, Python, JavaScript, C/C++, Go, etc.)
- **Git Diff Visualization**: Real-time visualization of changes compared to HEAD with color-coded line numbers (green for added, yellow for modified, red for deleted lines), deletion markers showing count of deleted lines
- **Search and Replace**: Text search with case-sensitivity support and replacement of found matches
- **Edit History**: Undo and Redo actions
- **Clipboard**: Copy, cut, and paste via system clipboard
- **Auto-save**: Prompt to save when closing a file with unsaved changes
- **Word Navigation**: Move cursor by words with Ctrl+Left/Right, select by words with Ctrl+Shift+Left/Right
- **Auto-Indentation**: New lines automatically inherit the indentation of the current line; smart indent adds an extra level after `{`, `(`, `[`, `:`
- **Auto-Close Brackets**: Automatically insert matching closing brackets and quotes (`()`, `[]`, `{}`, `""`, `''`); typing a closing bracket skips over existing one; backspace between a pair deletes both

## Navigation

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `↑` / `↓`         | Move cursor up/down                        |
| `←` / `→`         | Move cursor left/right                     |
| `Home`            | Go to beginning of line                    |
| `End`             | Go to end of line                          |
| `PageUp` / `PageDown` | Scroll by one page                      |
| `Ctrl+Home`       | Go to beginning of document                |
| `Ctrl+End`        | Go to end of document                      |
| `Ctrl+Left`       | Move cursor to previous word               |
| `Ctrl+Right`      | Move cursor to next word                   |
| `Ctrl+Shift+Left` | Select to previous word                    |
| `Ctrl+Shift+Right`| Select to next word                        |

## Editing

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+S`          | Save file                                  |
| `Ctrl+Shift+S`    | Save As (with executable checkbox)         |
| `Ctrl+Z`          | Undo last action                           |
| `Ctrl+Y`          | Redo undone action                         |
| `Ctrl+D`          | Duplicate current line or selection        |
| `Ctrl+G`          | Go to line number                          |
| `Backspace`       | Delete character to the left of cursor     |
| `Delete`          | Delete character to the right of cursor    |
| `Enter`           | Insert new line (with auto-indentation)    |
| `Tab`             | Insert indent (configurable, default 4)    |

## Search and Replace

### Interactive Search Modal (Ctrl+F)

Press `Ctrl+F` to open an interactive search modal with live preview:

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+F`          | Open search modal                          |
| Type text         | Live search updates as you type            |
| `Tab`             | Go to next match                           |
| `Shift+Tab`       | Go to previous match                       |
| `F3`              | Go to next match                           |
| `Shift+F3`        | Go to previous match                       |
| `Enter`           | Close modal, keep current match selected   |
| `Escape`          | Close search modal                         |
| Mouse click       | Click navigation buttons or `[X]` to close |

**Features:**
- Live search preview as you type
- Match counter display (e.g., "3 of 12")
- Navigation buttons: ◄ Prev, Next ►
- `[X]` close button in modal title
- Search query is preserved when modal is closed

**Search behavior outside modal:**
- `F3` / `Shift+F3` - Navigate through matches with modal closed
- `Tab` / `Shift+Tab` - Navigate matches when search is active
- Any navigation/editing key - Deactivates search mode
- Reopening with `F3` restores the last search query

### Interactive Replace Modal (Ctrl+H)

Press `Ctrl+H` to open an interactive replace modal with two input fields:

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+H`          | Open replace modal                         |
| Type in Find      | Live search updates as you type            |
| `Tab`             | Next match (in Find) or move to Replace field |
| `Shift+Tab`       | Previous match (in Find) or move to Find field |
| `Up` / `Down`     | Navigate between Find and Replace fields   |
| `F3`              | Go to next match                           |
| `Shift+F3`        | Go to previous match                       |
| `Enter`           | Replace current match and move to next     |
| `Escape`          | Close replace modal                        |
| Mouse click       | Click buttons (Replace, All, Prev, Next) or `[X]` |

**Features:**
- Two input fields: Find and Replace
- Live search preview as you type in Find field
- Match counter display (e.g., "3 of 12")
- Four buttons: Replace, All, ◄ Prev, Next ►
- `[X]` close button in modal title
- Both find and replace text are preserved when modal is closed

**Replace button actions:**
- **Replace** (`Ctrl+R`) - Replace current match and move to next
- **All** (`Ctrl+Alt+R`) - Replace all matches, show count, and close modal
- **◄ Prev** - Navigate to previous match
- **Next ►** - Navigate to next match

**Replace All Feedback:**
- After using "Replace All", the status bar shows how many replacements were made
- Example: "Replaced 5 occurrences"

## Clipboard

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+C`          | Copy selected text                         |
| `Ctrl+X`          | Cut selected text                          |
| `Ctrl+V`          | Paste from system clipboard                |

## Mouse Support

- **Single click**: Set cursor to click position
- **Double click**: Select word under cursor
- **Triple click**: Select entire line
- **Hold + move**: Text selection
- **Scroll wheel**: Scroll editor content

**Note:** Mouse selection works correctly in word wrap mode, accounting for wrapped lines.

## Word Wrap

When word wrap is enabled (configurable in settings), long lines are automatically wrapped to fit the panel width. The editor properly handles:

- **Cursor positioning**: Cursor navigation and display work correctly across wrapped lines
- **Mouse selection**: Clicks and drags accurately select text even when lines span multiple visual rows
- **Line numbers**: Displayed for logical lines, not visual rows
- **Editing operations**: All editing commands (cut, copy, paste, undo/redo) work seamlessly with wrapped content

Enable/disable word wrap in your configuration file (`~/.config/termide/config.toml`):
```toml
[editor]
word_wrap = true  # or false
```

## Auto-Indentation

When auto-indentation is enabled (default), pressing `Enter` creates a new line that inherits the indentation of the current line. Additionally, smart indent adds an extra level of indentation after lines ending with `{`, `(`, `[`, or `:`.

**Example:**
```
if condition {|        ← cursor here, press Enter
    |                  ← new line with extra indent
```

Enable/disable auto-indentation in your configuration file:
```toml
[editor]
auto_indent = true  # or false (default: true)
```

## Auto-Close Brackets

When auto-close brackets is enabled (default), typing an opening bracket or quote automatically inserts the matching closing character and places the cursor between them.

**Supported pairs:** `()`, `[]`, `{}`, `""`, `''`

**Behavior:**
- Typing `(` inserts `()` with cursor between them
- Typing `)` when the character at cursor is already `)` skips over it instead of inserting a duplicate
- Pressing `Backspace` between an empty pair (e.g., `()`) deletes both characters
- Quotes are not auto-closed after alphanumeric characters (to support apostrophes in words like "it's", "don't")

Enable/disable auto-close brackets in your configuration file:
```toml
[editor]
auto_close_brackets = true  # or false (default: true)
```

## Status Bar Information

When working in the editor, the status bar displays:
- File name and modification indicator (*)
- Current cursor position (line:column)
- Search information (number of matches)
- File type (plain text / read-only)

## Git Diff Visualization

When editing files in a git repository with `show_git_diff` enabled, the editor displays real-time diff information compared to HEAD:

### Line Number Colors

Line numbers are color-coded to show the status compared to HEAD:

- **Green** - Line was added (not in HEAD)
- **Yellow** - Line was modified (changed from HEAD)
- **Red marker (▶)** - Marks a deletion point (lines were deleted after this line)
- **Default color** - Line unchanged from HEAD

### Deletion Markers

When lines are deleted, a virtual line is inserted to visualize the deletion:

- Displays a horizontal line (`━`) spanning the editor width
- Shows deletion marker character (`▶`) in the line number area with red color
- Displays centered text: "N lines deleted" (e.g., "3 lines deleted")
- Styled in gray/disabled color to distinguish from actual content
- Does not affect line numbering (shows `▶` instead of a number)

**Example:**
```
  42 | function calculateTotal() {
 ▶   | ━━━━━━━ 5 lines deleted ━━━━━━━
  43 |     return result;
```

### How It Works

- **Automatic updates**: Diff updates when you save the file
- **Real-time comparison**: Compares current buffer content with HEAD version
- **Undo/Redo support**: Markers appear/disappear as you undo/redo deletions
- **Works with editing**: All normal editing operations work seamlessly with diff visualization

### Configuration

Enable or disable git diff visualization in your configuration file (`~/.config/termide/config.toml`):

```toml
# Show git diff colors on line numbers (default: true)
show_git_diff = true
```

**Notes:**
- Only works when editing files within a git repository
- Requires the file to exist in HEAD (new untracked files show all lines as added)
- Virtual deletion marker lines are visual-only and don't affect the file content

## LSP (Language Server Protocol)

TermIDE includes built-in LSP support for intelligent code assistance. When a language server is configured and available, you get:

- **Code Completion** - Context-aware suggestions as you type
- **Diagnostics** - Real-time error and warning indicators
- **Loading Status** - Spinner in panel title shows server status (starting/indexing)

### Triggering Completion

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+.`          | Manually trigger completion popup          |
| `Enter`           | Accept selected completion                 |
| `Escape`          | Close completion popup                     |
| `↑` / `↓`         | Navigate through suggestions               |
| Type characters   | Filter suggestions by typing               |

**Auto-completion:** When enabled (default), completion popup appears automatically:
- After typing identifier characters (letters, numbers, `_`)
- Immediately after trigger characters (`.`, `:`, `(`, `<`)

### Completion Popup

The completion popup displays:
- **Icon** - Indicates item type (function ƒ, variable v, class C, etc.)
- **Label** - The completion text
- **Scroll indicators** - ▲/▼ when list is scrollable

### Configuration

Configure LSP in your configuration file (`~/.config/termide/config.toml`):

```toml
[lsp]
enabled = true              # Enable/disable LSP (default: true)
auto_completion = true      # Auto-trigger completion on typing (default: true)
completion_delay_ms = 100   # Delay before auto-completion (default: 100)

[lsp.servers.rust]
command = "rust-analyzer"
args = []
root_markers = ["Cargo.toml"]

[lsp.servers.python]
command = "pylsp"
args = []
root_markers = ["pyproject.toml", "setup.py", "requirements.txt"]

[lsp.servers.typescript]
command = "typescript-language-server"
args = ["--stdio"]
root_markers = ["package.json", "tsconfig.json"]
```

### Server Status Indicator

When opening a file with LSP support, the panel title shows a loading spinner:
- `⠋ main.rs (starting)` — Server is starting
- `⠋ main.rs (indexing)` — Server is indexing the project
- `main.rs` — Server is ready

### Supported Languages

LSP works with any language server that implements the LSP protocol. Common examples:
- **Rust** - rust-analyzer
- **Python** - pylsp, pyright
- **TypeScript/JavaScript** - typescript-language-server
- **Go** - gopls
- **C/C++** - clangd

**Note:** You need to install the language server separately. TermIDE only provides the LSP client integration.

## Vim Mode

TermIDE includes an optional Vim-style editing mode with full Cyrillic keyboard support.

### Enabling Vim Mode

Enable Vim mode in your configuration file (`~/.config/termide/config.toml`):

```toml
vim_mode = true
```

### Available Modes

| Mode | Description | Indicator |
|------|-------------|-----------|
| Normal | Navigation and commands | (none) |
| Insert | Text input | `-- INSERT --` |
| Visual | Character selection | `-- VISUAL --` |
| Visual Line | Line selection | `-- VISUAL LINE --` |

### Mode Switching

| Key | Action |
|-----|--------|
| `Escape` | Return to Normal mode |
| `i` | Insert before cursor |
| `I` | Insert at line beginning |
| `a` | Append after cursor |
| `A` | Append at line end |
| `o` | Open line below |
| `O` | Open line above |
| `v` | Enter Visual mode |
| `V` | Enter Visual Line mode |

### Motion Keys (Normal/Visual)

| Key | Action |
|-----|--------|
| `h` / `←` | Move left |
| `j` / `↓` | Move down |
| `k` / `↑` | Move up |
| `l` / `→` | Move right |
| `w` | Next word start |
| `b` | Previous word start |
| `e` | Next word end |
| `0` | Line start |
| `$` | Line end |
| `^` | First non-blank character |
| `gg` | Go to first line |
| `G` | Go to last line |
| `{number}G` | Go to line number |

### Operators (Normal Mode)

| Key | Action |
|-----|--------|
| `d` | Delete (+ motion) |
| `dd` | Delete line |
| `D` | Delete to end of line |
| `c` | Change (+ motion) |
| `cc` | Change line |
| `C` | Change to end of line |
| `y` | Yank/copy (+ motion) |
| `yy` | Yank line |
| `p` | Paste after cursor |
| `P` | Paste before cursor |
| `x` | Delete character |
| `r` | Replace character |
| `u` | Undo |
| `Ctrl+R` | Redo |

### Cyrillic Keyboard Support

Vim mode works seamlessly with Cyrillic keyboard layouts. When typing in Russian or other Cyrillic layouts, all Vim commands are automatically translated:

- `о` (Russian) → `j` (move down)
- `л` (Russian) → `k` (move up)
- `н` (Russian) → `y` (yank)
- `в` (Russian) → `d` (delete)

This allows you to use Vim commands without switching keyboard layouts.

### Visual Mode Operations

In Visual or Visual Line mode:
- Use motion keys to extend selection
- `d` or `x` - Delete selection
- `y` - Yank selection
- `c` - Change selection (delete and enter Insert mode)
- `>` - Indent selection
- `<` - Unindent selection

### Search in Vim Mode

| Key | Action |
|-----|--------|
| `/` | Search forward |
| `?` | Search backward |
| `n` | Next match |
| `N` | Previous match |

**Note:** Standard editor shortcuts (`Ctrl+S`, `Ctrl+Z`, etc.) continue to work in all Vim modes.
