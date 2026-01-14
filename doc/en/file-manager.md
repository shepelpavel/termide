# File Manager

The file manager panel provides an intuitive interface for navigating the file system and performing operations on files and directories.

## Navigation

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `↑` / `↓`         | Move cursor up/down                        |
| `Enter`           | Enter directory, preview media, or open file |
| `Backspace`       | Go to parent directory                     |
| `~`               | Go to home directory                       |
| `PageUp` / `PageDown` | Scroll list by one page                |
| `Home` / `End`    | Go to beginning/end of list                |
| `Tab`             | Go to next panel                           |
| `Shift+Tab`       | Go to previous panel                       |

## File Selection

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Insert`          | Toggle selection of current file           |
| `Shift + ↑ / ↓`   | Select multiple consecutive files          |
| `Ctrl+A`          | Select all files and directories in panel  |
| `Escape`          | Clear all selections                       |

## File Operations

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+N` / `F`    | Create new file                            |
| `D` / `F7`        | Create new directory                       |
| `Delete` / `F8`   | Delete selected files/directories          |
| `C` / `F5`        | Copy selected files/directories            |
| `M` / `F6`        | Move/rename files/directories              |
| `F4`              | Open file in editor                        |
| `Ctrl+R`          | Refresh current directory contents         |
| `Space`           | Show file/directory information            |

## Search

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+F`          | Search files by name (glob patterns)       |
| `Ctrl+Shift+F`    | Search in file contents (regex)            |

### File Search (Ctrl+F)

Opens a modal for quick file search by name using glob patterns:
- Type to filter files in real-time
- Results show relative paths with git status colors
- Press Enter to open selected file
- Press Escape or click outside to close

### Content Search (Ctrl+Shift+F)

Opens a modal for searching text within files using regular expressions:
- Searches only in text files (binary files are skipped)
- Large files are skipped (configurable limit in settings)
- Results show file path, line number, and context (3 lines)
- Matched text is highlighted
- Press Enter to open file at the matched line
- Press Escape or click outside to close

## Clipboard

| Shortcut           | Action                                     |
|-------------------|--------------------------------------------|
| `Ctrl+C`          | Copy paths of selected items               |
| `Ctrl+X`          | Cut paths of selected items                |
| `Ctrl+V`          | Paste files from clipboard                 |

## Git Integration

The file manager displays file status in Git repositories, highlighting new, modified, and deleted files.

## Media Preview

The file manager can preview images and videos using console image viewers.

**File opening logic:**

| File type | Action |
|-----------|--------|
| Raster images (PNG, JPG, JPEG, GIF, WebP, BMP, TIFF) | ImagePanel (native graphics) or xdg-open fallback |
| Vector images (SVG, ICO) | xdg-open (system viewer) |
| Videos (MP4, MKV, AVI, MOV, WebM, FLV, WMV, M4V) | xdg-open (system player) |
| Binary files | xdg-open (system default) |
| Text files | Editor panel |
| Executable files | Run in terminal |

**Shortcuts:**
- `Enter` → smart open (see table above)
- `F3` → view file (like Enter, but executables open in editor instead of running)
- `Shift+Enter` → force open with xdg-open (system default application)
- `F4` → always open in editor

**Native Graphics:**
termide automatically detects if the parent terminal supports graphics protocols (Kitty, Sixel, iTerm2). When supported, raster images are rendered directly in the ImagePanel without external tools.

**Supported terminals:**
- Kitty, WezTerm, iTerm2, Ghostty, foot - full graphics support
- Other terminals - fallback to xdg-open

## Mouse Support

- **Single click**: Select a file or directory
- **Double click**: Enter directory or open file
- **Scroll wheel**: Scroll through file list
