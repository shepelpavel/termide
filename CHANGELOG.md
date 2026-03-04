# Changelog

All notable changes to TermIDE will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.14.1] - 2026-03-04

### Added
- **File Manager**: F3 opens files in read-only mode (view mode)
- **File Manager**: Directory picker on title click for quick navigation
- **File Manager**: Keyboard shortcuts for rename (R/F2), view (V/F3), edit (E/F4), new file (F/Ctrl+N)
- **Git Status**: Keyboard shortcuts for revert, view, edit
- **Layout**: Improved accordion panel insertion and removal behavior
- **Git Status**: Auto-fetch on init to show correct pull button

### Changed
- **I18n**: `init()` and `init_with_language()` return `Result` instead of panicking
- **Codebase**: Added `must_use` attributes, fixed unsafe unwrap

### Fixed
- **Terminal**: Resize alternate screen buffer on window resize
- **Git**: Canonicalize paths before strip_prefix for symlink repos
- **File Manager**: Deleted files appearing in wrong directories
- **Keyboard**: Fixed incorrect and missing keyboard shortcuts in documentation

## [0.14.0] - 2026-02-25

### Added
- **UI**: Replace home directory prefix with `~` in terminal title, FileManager panel titles, sessions modal, and directory switcher
- **UI**: Update terminal window title when switching or creating sessions
- **Editor**: Ctrl+Up/Down for paragraph/symbol navigation

### Changed
- **UI**: Deduplicate dropdown hit-test logic, fix swallowed errors in menu rendering
- **Panel Operations**: Auto-close operations panel when it becomes empty

## [0.13.0] - 2026-02-24

### Added
- **Editor**: Auto-closing brackets and quotes with context-aware logic (don't close apostrophes mid-word)
- **Editor**: Ctrl+Left/Right word navigation and Ctrl+Shift+Left/Right word selection
- **Editor**: Split-bracket indent on Enter between matching pairs (`{|}` → three lines with proper indentation)
- **Editor**: `auto_indent` and `auto_close_brackets` configuration options

### Changed
- **VFS**: Deduplicate SFTP operations via `spawn_op`/`spawn_op2` helpers (-224 LOC)
- **I18n**: Replace unsafe pointer cast in `t()` with safe `Box::leak` approach
- **I18n**: Use fallback instead of guard-checked `expect` in pluralization
- **Editor**: Extract word boundary logic from vim motions into shared `word_boundary` module

## [0.12.6] - 2026-02-23

### Added
- **CLI**: `--log-level`, `--no-lsp`, `--config` command-line arguments
- **CLI**: Set terminal window/tab title to "Termide: <cwd>" on startup
- **Logger**: TRACE log level and trace user input events

### Changed
- **Git Status**: Remove flat view mode, always use tree view
- **ProgressModal**: Deduplicate 8 constructors via shared base + struct update syntax (-166 LOC)
- **API**: Return `&[T]` instead of `&Vec<T>` in `PanelGroup::panels()` and `TerminalScreen::get_line_by_absolute()`
- **Constants**: Centralize `MEGABYTE` in `termide-config`, remove duplicates from `panel-editor` and `modal`
- **Codebase**: Remove unused `_buffer` parameter from `build_diagnostic_maps`
- **Codebase**: Remove redundant clones, deduplicate constructors, fix unwrap

### Fixed
- **Editor**: Invalidate wrap cache on selection delete
- **Journal**: Account for trailing empty line in auto-scroll check
- **Journal**: Bind auto-scroll to cursor position instead of key codes
- **Terminal**: Update cursor position on arrow key movement

## [0.12.5] - 2026-02-18

### Added
- **Git Status**: Auto-switch between tree and flat view based on panel width (>= 35 columns)

### Changed
- **Codebase**: Extract `TREE_VIEW_MIN_WIDTH` and `MAX_RECURSION_DEPTH` shared constants
- **Codebase**: Consolidate `SpeedTracker` into `file-ops` crate, re-export from `state`
- **Terminal**: Deduplicate constructor code via `set_env`/`spawn_reader`/`build` helpers
- **Git Status**: Optimize tree prefix computation from O(n²) to O(n) reverse scan
- **Clipboard**: Graceful fallback on headless systems instead of panic

### Fixed
- **Editor**: Correct cursor positioning with wide characters at wrap boundary and in no-wrap mode
- **Session**: Prevent duplicate unsaved buffer files on auto-save

## [0.12.4] - 2026-02-17

### Added
- **Outline**: YAML and XML regex fallback for outline panel when tree-sitter is unavailable

### Changed
- **Editor**: Extract viewport/scrolling logic into dedicated `editor_viewport.rs` module (-572 LOC from core.rs)
- **Editor**: Extract generic `poll_receiver` helper to deduplicate LSP polling methods
- **Codebase**: Replace magic numbers with named constants (`PROGRESS_THROTTLE_FILES`, `EXECUTABLE_MASK`)
- **Codebase**: Preserve error context in VFS lock/channel/FTP operations instead of discarding
- **Codebase**: Replace `is_none() + unwrap()` pattern with `is_none_or` and double `.all()` iteration with single-pass check

### Fixed
- **Git**: Show local commit count as ahead when no remote/upstream is configured
- **Git Status**: Re-discover git repository on refresh after external `git init`
- **Session**: Restore non-empty orphaned unsaved buffers instead of deleting them
- **Editor**: Invalidate word-wrap cache on undo/redo, replace, and vim edits

## [0.12.3] - 2026-02-09

### Added
- **Status Bar**: Display LF/CRLF line ending indicator before encoding (UTF-8) in editor status bar

### Changed
- **Codebase**: Deduplicate `LineEnding` enum — single definition in buffer crate root, reused internally

### Fixed
- **Buffer**: Normalize CRLF line endings to LF on file load, preserving original type for save
- **Editor**: Normalize line endings on paste for VTE-based terminals
- **Terminal**: Add Alt+Enter as fallback for newline input on VTE terminals
- **Codebase**: Reduce duplication and improve safety across VFS and rendering modules

## [0.12.2] - 2026-02-08

### Added
- **i18n**: Complete Chinese (zh) localization — translated all remaining English UI strings
- **i18n**: Translated git action labels (stage, unstage, commit, etc.) across all locales
- **Documentation**: Added full Chinese documentation set (doc/zh/) and README.zh.md

### Changed
- **Documentation**: Updated English and Russian docs with new keybindings, menu items, panel types, LSP/Vim sections, and modernized config example

### Fixed
- **Outline Panel**: CJK headings no longer truncated to first character — use display width instead of character index for positioning

## [0.12.1] - 2026-02-08

### Added
- **File Manager**: Toggle hidden files visibility with `.` key

### Changed
- **InfoModal**: Colored segments, multiline support and truncation
- **Codebase**: Rename Welcome remnants to Help across codebase

### Fixed
- **Layout**: Refresh stale panels on next/prev switch in group — fixes Outline going empty after panel switching
- **File Manager**: Treat directory symlinks as directories
- **App**: Enable search in vim normal mode and JournalPanel
- **Outline**: Repopulate outline after expanding collapsed panel

## [0.12.0] - 2026-02-07

### Added
- **Outline Panel**: Structural code navigation using tree-sitter queries with regex fallback for markdown/HTML; tree-drawing prefixes, cursor sync, and keyboard/mouse navigation
- **Global Hotkeys**: Keyboard shortcuts for Outline, Diagnostics, and Git Log panels
- **File Manager Edit Button**: Replaced Git Status button with Edit in file info modal

### Changed
- **Code Quality**: Eliminated unsafe unwraps across app and git crates, deduplicated batch operations
- **Modal Handlers**: Deduplicated modal handlers, added path traversal guard and RwLock recovery
- **Panel Index Cleanup**: Removed obsolete panel_index field, fixed related clippy warnings

### Fixed
- **Outline Sync**: Outline panel now syncs on panel navigation and debounces live edits
- **Outline Resync on Close**: Outline panel rebinds to another editor (or clears) when tracked editor is closed

## [0.11.3] - 2026-02-06

### Added
- **4 New Themes**: Matrix (digital rain), Terminator (Skynet HUD), Pip-Boy (Fallout CRT), Manuscript (medieval parchment)
- **Adaptive Default Layout**: Width-dependent session layout — wide terminals (>= 160 cols) get 3-group layout, normal terminals get 2-group layout with sidebar accordion
- **GitStatus in Default Sidebar**: New sessions automatically include GitStatus panel in the sidebar accordion when a git repository is detected
- **Diagnostic Row Caching**: Editor caches diagnostic row counts for faster viewport scrolling with LSP diagnostics

### Changed
- **State Module Split**: Refactored monolithic `crates/state/src/lib.rs` into submodules: `batch`, `layout`, `operations`, `pending_action`, `ui`
- **Dead Code Removal**: Removed unused functions across 7 crates (`clipboard::has_text`, `git::check_git_available`, `logger::get_entries`, `editor::cleanup_temp_file`, `theme::all_themes`, `theme::builtin_theme_names`, `system_monitor::cpu_usage_float`)
- **Removed text-search Crate**: Deleted unused `termide-text-search` crate from workspace
- **Documentation Update**: Updated README, architecture docs, and developer guides (EN+RU) to reflect current workspace structure, 24 themes, 15 languages, all panel types

### Fixed
- **Security**: Updated `time` crate 0.3.45 → 0.3.47 to fix stack exhaustion DoS vulnerability (GHSA)
- **Editor Diagnostics**: Reverted diagnostic row deduplication in viewport scroll methods that caused incorrect cursor positioning
- **Watcher Registration**: Fixed watchers not being registered on session switch; optimized `.git/objects` watch exclusion
- **Terminal Link Highlight**: Fixed Ctrl+click link highlight offset for non-ASCII content in terminal panel
- **File Manager Git Root**: Preserved `git_root` on internal navigation, fixed post-commit refresh losing git context
- **File Manager Redraw**: Fixed missing redraw after async git status update completed

## [0.11.2] - 2026-02-04

### Fixed
- **UTF-8 Panic**: Fixed byte-index panic when resizing panels with Cyrillic/Unicode text — now truncates by display width, not byte offset
- **Editor Word-Wrap Mouse**: Fixed cursor mispositioning on mouse click in word-wrap mode when git deletion markers are present
- **Editor Word-Wrap Scroll**: Fixed viewport failing to scroll past lines with deletion markers or diagnostics in word-wrap mode
- **Editor Visual Cursor**: Fixed cursor unable to reach end of last visual row during page navigation in word-wrap mode
- **Editor Inline Diff Click**: Fixed mouse click offset in no-wrap mode when inline diff deleted text shifts visual positions
- **Git Status Stale Panel**: Collapsed git-status panel now marks itself stale and refreshes on expand (MarkStale/RefreshIfStale)
- **Git Status Title**: Collapsed git-status panel title now updates branch/file counts on git/filesystem events
- **Git Diff Toggle**: Mouse click on file header in git-diff panel now correctly toggles collapse
- **Operations Panel Focus**: Opening the operations panel no longer steals focus from the active editor
- **CodeQL Security**: Broke taint chain for cleartext-logging alerts in VFS types

## [0.11.1] - 2026-02-01

### Added
- **109 New Tests**: Added tests across 8 crates covering critical gaps (layout, config, buffer, clipboard, core, git, i18n, session)

### Changed
- **Stale-on-Collapse Optimization**: Collapsed panels skip tick/watcher/git/LSP background work; refresh once when expanded again
- **Expanded-Aware Iterators**: Added `iter_all_panels_with_expanded_state_mut()` and `iter_expanded_panels_mut()` to LayoutManager
- **LSP Polling Scoped to Expanded Editors**: Loading status updates only run for visible editor panels

### Fixed
- **CI/Security**: Vendored OpenSSL for cross-platform builds, fixed macOS statvfs types, redacted credentials from logs
- **SFTP Stale Bug**: Remote panels are never marked stale by local fs/git events (prevents broken reconnections)
- **VFS Spinner Hang**: FileManager tick always drains VFS and git-status receivers even when collapsed, preventing stuck spinners
- **VFS Error Draining**: Fixed early return in FileManager tick that prevented VFS error results from being consumed

## [0.11.0] - 2026-02-01

### Added
- **VFS with SFTP Support**: Remote file operations via sftp:// protocol — browse, open, edit, upload, download remote files
- **Unified File Operations System**: OperationManager with pause/cancel support for copy, move, delete, upload, download
- **Operations Panel**: Dedicated panel showing active file operations with progress cards and speed tracking
- **Conflict Resolution**: End-to-end conflict handling — overwrite, skip, rename, overwrite-all, skip-all
- **Byte Progress Bar**: Single-file upload/download progress with byte-level granularity
- **Background Operations Indicator**: Status bar indicator for running file operations
- **Ctrl+C Copy in Terminal**: Copy selected text with Ctrl+C when selection is active
- **Auto-scroll During Selection**: Continuous auto-scroll when dragging mouse beyond editor/terminal viewport
- **Word Wrap Caching**: Cached word wrap functions for faster cursor movement in large files
- **Text Selection in Modals**: Full text selection support in all input fields
- **Word Wrap in Journal**: JournalPanel now supports word-wrapped log entries
- **Version Header in Help**: Help panel displays current version
- **Mouse Scroll Coalescing**: Batched scroll events reduce render cycles during fast scrolling

### Changed
- **i18n Cleanup**: Removed 98 dead translation keys, wired 30 new keys for previously hardcoded UI strings
- **OperationManager Migration**: Migrated local copy, delete, batch upload/download, and directory copy to unified system
- **Logging Migration**: Switched to standard `log` crate macros throughout the codebase
- **Module Decomposition**: Split monolithic `app/mod.rs` into focused submodules (watcher, session, background_ops, etc.)
- **SuggestionInput Component**: Extracted reusable dropdown input widget from bookmark modal
- **Zero-alloc Rendering**: Reduced allocations in rendering hot paths with static buffers
- **Adaptive Tick Rate**: Event loop slows from 24 FPS to 5 FPS after 500ms idle, reducing CPU to near-zero
- **Conditional System Monitor Redraw**: Only redraws status bar when CPU% or memory MB actually changes
- **Watcher Registration**: Split from per-tick polling; registration runs only on panel add/navigate

### Fixed
- **Operations Panel Tracking**: Fixed batch and remote operation tracking in operations panel
- **Conflict Reuse**: Fixed conflict resolution reuse and batch progress tracking
- **Duplicate Downloads**: Prevented duplicate downloads when copying from remote
- **Editor Upload Flow**: Fixed close-after-save and file manager refresh for remote saves
- **Session Panel Widths**: Adapt panel widths to current terminal size on session restore
- **Terminal Duplicate Prompts**: Prevented duplicate prompts during sync_output transitions
- **Editable Select Padding**: Fixed extra empty lines and padding in dropdown
- **VFS Path Sync**: Fixed local directory navigation path synchronization
- **Git Session Restore**: Preserved repository list when restoring session with submodule
- **Unicode Search Highlighting**: Corrected Unicode handling in editor search
- **Spinner CPU Usage**: Throttled spinner redraws to reduce idle CPU
- **Bookmark Navigation**: Fixed dropdown keyboard navigation and mouse click handling

## [0.10.1] - 2026-01-23

### Changed
- **LSP Performance**: Reduced lock contention and reorganized response handling
- **Editor Refactoring**: Extracted mouse handling to separate module
- **Modal Cleanup**: Removed unused panel_index field from SelectOption
- **Theme Colors**: Updated theme color usage in modals and dropdowns

### Fixed
- **Terminal Sync Artifacts**: Defer cache invalidation during sync_output batches to prevent partial frame rendering
- **Terminal Render Clearing**: Clear render area before drawing to prevent visual artifacts
- **Modal Width Stability**: Fixed editable select width during dropdown toggle
- **Terminal Cache**: Added cache invalidation for all buffer-modifying operations

## [0.10.0] - 2026-01-23

### Added
- **Vim Mode**: Full vim keybinding support for Editor panel with Cyrillic keyboard layout support
- **Directory Switcher**: Quick directory navigation modal with Ctrl+P, recent directories, and fuzzy search
- **Bookmark Add Modal**: Smart group input with dropdown suggestions for organizing bookmarks
- **Modal Paste Support**: Paste text directly into modal input fields with control character filtering

### Fixed
- **Terminal Visual Artifacts**: Fixed race condition in sync_output cache invalidation
- **Terminal Batch Rendering**: Force cache invalidation on ED (clear screen) commands
- **Buffer Invariant**: Added ensure_buffer_size() to prevent IL/DL edge cases

### Documentation
- Added documentation for vim mode, directory switcher, and bookmarks features

## [0.9.1] - 2026-01-22

### Added
- **Linux TTY Support**: Native terminal colors for Linux console/framebuffer (TERM=linux)
- **Remote Branch Display**: Git Status panel shows remote branches not available locally
- **DECSTBM Scroll Regions**: Proper CSI scroll region support for terminal applications

### Changed
- **Terminal Refactoring**: Extracted utility modules (clipboard, disk_space, link_detection, shell_utils)
- **Editor Refactoring**: Decomposed core.rs into focused modules (LSP, movement, search, text)
- **Git Refactoring**: Split monolithic lib.rs into command, commits, files, operations modules
- **Performance**: Cached regex patterns for link detection, reduced panel code duplication
- **DRY Improvements**: Extracted helpers for file/terminal opening across panels

### Fixed
- **Synchronized Output**: Fixed cache invalidation when exiting sync_output mode (CSI ? 2026 l)
- **CI Release Notes**: Fixed extraction of release notes from CHANGELOG.md

### Performance
- **Terminal**: Reduced SSH input latency and htop rendering overhead

## [0.9.0] - 2026-01-20

### Added
- **LSP Integration**: Full Language Server Protocol support with completion, diagnostics, hover, and go-to-definition
- **Git Diff Panel**: GitHub-style diff viewer with commit diff viewing from Git Log panel
- **Save As Modal**: New dialog with executable checkbox for setting file permissions (Ctrl+Shift+S)
- **Report Scripts**: Scripts with `.report.` suffix run in background and show output in modal dialog
- **Runtime Language Switching**: Change UI language on the fly with 15 language support
- **Dynamic Help Panel**: Replaced static WelcomePanel with context-aware help
- **Git Status Enhancements**: Init button when no repository, left-ellipsis path truncation, enhanced file properties
- **Terminal Features**: File path detection, multi-line link highlighting, bracketed paste support
- **Editor Features**: Disk space info in status bar, improved PageUp/PageDown behavior
- **UI Improvements**: Smart title truncation with Unicode ellipsis, unified animated spinners

### Changed
- **Menu Renamed**: "Tools" → "Windows", "Actions" → "Scripts" for better UX
- **Keybindings**: Ctrl+Shift+S now opens Save As (was Force Save)
- **Submenu Renamed**: "Manage actions" → "Manage scripts"
- **Git Diff Style**: Unified file headers with termide design system
- **Refactoring**: Extensive code cleanup with Command Pattern, state extraction, and helper consolidation

### Fixed
- **i18n**: Format keys moved to correct section, warnings redirected to journal
- **Git**: Submodule detection by parsing .gitmodules
- **Copy/Move Modals**: Removed duplicate text

### Documentation
- **LSP Docs**: Added comprehensive LSP documentation with configuration examples
- **Scripts Docs**: Updated for new menu names and .report. suffix feature
- **Editor Docs**: Added Ctrl+Shift+S Save As keybinding

## [0.8.9] - 2026-01-15

### Added
- **Terminal URL detection**: Ctrl+hover highlights URLs (cyan + underline), copies to clipboard, Ctrl+click opens in browser
- **ImagePanel reuse**: Browsing images reuses existing panel instead of creating new ones
- **Git async operations**: Push/Pull run in background with spinner indicator and result modal

### Fixed
- **Modal UX**: InfoModal closes only on Enter, Escape, or button click (not any key press)
- **Modal display**: Git operation results show actual output without "Error:"/"Status:" labels
- **Unicode click detection**: Fixed button click detection for Unicode text using grapheme width
- **Git TUI corruption**: Stdout/stderr piped to prevent git output from corrupting terminal display

### Changed
- **Git spinner location**: Moved from status bar to GitStatusPanel for better visibility during panel switches
- **UI cleanup**: Consolidated duplicate code and removed dead code
- **Menu structure**: Restructured menu and fixed log viewer truncation

## [0.8.8] - 2026-01-13

### Security
- **lru vulnerability**: Fixed GHSA-rhfx-m35p-ff5j (IterMut Stacked Borrows violation) by updating to lru 0.16.3

### Added
- **Configurable keybindings**: Full keybinding customization via config.toml for all panels
- **Shared abstractions**: New ClickTracker, Viewport, SelectionStyle utilities for DRY code

### Changed
- **Dependencies**: Updated ratatui 0.29→0.30, ratatui-image 5→10
- **Performance**: 50-500x speedup in hot paths (grapheme iteration, git diff markers, LRU eviction, gitignore checks)
- **Refactoring**: Integrated ClickTracker into GitStatusPanel and FileManager (~40 LOC saved)

## [0.8.7] - 2026-01-12

### Added
- **Install script**: Nix installation method with `nix profile install` for NixOS users
- **Install script**: Nix shown as "(recommended for NixOS)" on NixOS systems

### Fixed
- **Install script**: Works correctly when piped via `curl | sh` (reads from /dev/tty)
- **Install script**: Fixed download URLs (removed incorrect `v` prefix from version)
- **Install script**: Fixed ANSI escape codes appearing literally in success message
- **Install script**: Added `--refresh` flag to ensure latest version is installed via Nix

### Changed
- **Build**: Enabled LTO and strip for release builds (-14% binary size: 29→25 MiB)
- **Packaging**: Removed obsolete help/themes install steps (files are embedded in binary)
- **Nix**: Track Cargo.lock for reproducible flake builds

## [0.8.6] - 2026-01-11

### Added
- **i18n**: 25 missing translation keys for Git panel and preferences in 7 languages (de, es, fr, hi, pt, th, zh)

### Fixed
- **Theme dropdown**: Mouse clicks now work correctly for all items (not just first 12)
- **Terminal paste**: Removed explicit flush preventing React/Ink TUI app overflow errors (e.g., Claude Code)

### Changed
- **README**: Added "Why TermIDE?" comparison table and quick navigation links
- **README**: Fixed theme screenshot paths

## [0.8.5] - 2026-01-07

### Added
- **Universal install script**: New `install.sh` for easy cross-platform installation

### Fixed
- **Config hot-reload**: Editor now applies settings (theme, word_wrap, tab_size) when saving config.toml
- **UTF-8 paths**: Git Status panel correctly handles non-ASCII file paths in truncation

### Changed
- **Git repo discovery**: Repositories now discovered from panel paths instead of project root
- **Performance**: O(n²) → O(n log n) optimization in nested paths removal algorithm
- **Performance**: Reduced allocations in search replace using `take()` instead of `clone()`
- **Code quality**: Extracted helper methods in GitStatusPanel (git operations, rendering, navigation, mouse handling)
- **Code quality**: Removed unused `SearchDirection` and `Action::Group` from buffer module

## [0.8.4] - 2026-01-06

### Added
- **CI/CD automation**: Automatic AUR and Homebrew updates on release via GitHub Actions
- **AUR packages published**: `termide` (source) and `termide-bin` (binary) now available on AUR

### Fixed
- **Terminal text selection**: Selection now uses absolute buffer coordinates
- **Selection persistence**: Selection follows text when scrolling through scrollback
- **Ctrl+Shift+C**: Keyboard shortcut for copying terminal selection
- **Selection cleanup**: Selection cleared on keyboard input
- **Trailing whitespace**: Preserved when copying selected text

## [0.8.3] - 2026-01-01

### Fixed
- CI: Use Ubuntu 22.04 for Linux builds to ensure glibc compatibility with Debian 12

## [0.8.2] - 2026-01-01

### Added
- **Inline diff visualization**: Character-level diff highlighting for modified lines in editor (deleted text in red, inserted in green)
- **Git gutter markers**: Visual indicators in editor line numbers (`+` for added, `~` for modified lines)
- **Revert button**: Quick revert action for individual files in Git Status panel

### Fixed
- Git commands now properly detect repository when editing files in subdirectories
- Inline diff line mapping correctly tracks modified lines when other lines are added/deleted above
- Debian package (.deb) now includes binary in `/usr/bin/`
- README.md download links corrected for .deb and .rpm packages

## [0.8.1] - 2025-12-28

### Added
- **Alt+G hotkey**: Quick access to open/focus Git Status panel
- **Git Status navigation**: Spatial navigation with Up/Down between sections, Left/Right within rows
- **Git Status sticky headers**: Staged/Unstaged headers stay visible when scrolled
- **CommitModal improvements**: PageUp/PageDown support, mouse scroll in textarea
- **Terminal Shift+Enter**: Insert newline for multi-line input

### Fixed
- Git Status panel: PageDown now scrolls fully to show all files
- Terminal: Shift+Enter sends newline instead of CSI u escape sequence
- Editor: Mouse scroll no longer jumps back to cursor position
- CommitModal: Textarea border now uses correct accent color

### Documentation
- Added Shift+Enter shortcut to terminal documentation (EN/RU)
- Added Git panel menu item and Alt+G shortcut to UI docs (EN/RU)
- Updated help files for all 9 languages with Git panel shortcuts

## [0.8.0] - 2025-12-27

### Added
- **Git Status Panel**: Full git status panel with staged/unstaged files, commit/push/pull actions
- **Git Log Panel**: View commit history with navigation
- **Sessions Menu**: New/switch/change-root session actions via Ctrl+S menu
- **CommitModal**: Multi-line textarea for entering commit messages with Commit/Cancel buttons
- **TextArea component**: Multi-line text input with 2D cursor, selection, undo/redo, clipboard support
- **Input field improvements**: Text selection, undo/redo (Ctrl+Z/Y), clipboard support in modal input fields

### Fixed
- Modal input fields: UTF-8 handling, clipboard operations, text selection
- Collapsed panel titles: now show truncated title instead of empty string
- Git Status panel title: properly displays repo name, branch, and status indicators

### Changed
- Git Status panel: spinner icon during refresh, status numbers moved to panel title
- InlineSelector component: improved styling and interaction

## [0.7.0] - 2025-12-18

### Added
- **ImagePanel**: Native image preview using terminal graphics protocols (Kitty, Sixel, iTerm2)
- **Panel resize**: Drag-and-drop panel border resizing with mouse
- **F3 key**: View file without executing (executables open in editor)
- **Shift+Enter**: Force open file with system application (xdg-open)
- **Execute on Enter**: Run executable files directly in terminal
- Smart file type detection: raster images → ImagePanel, vector/video/binary → xdg-open

### Fixed
- Sessions list: cursor now positions on current session
- Editor: off-by-one error in smart word wrap
- SaveAs dialog: now uses file manager's working directory

### Changed
- Split `min_panel_width` config into separate parameters for finer control
- Binary file detection extracted to shared utility (`core::util::is_binary_file`)

### Documentation
- Updated file manager docs with new keyboard shortcuts (F3, Shift+Enter)
- Updated help screens for all 9 languages

## [0.6.1] - 2025-12-16

### Added
- Live theme preview: theme changes on cursor navigation in theme selection menu
- Theme restored on cancel (Esc), saved on confirm (Enter)

### Fixed
- Menu toggle on repeated click (Preferences dropdown now closes on second click)
- Nested submenu toggle (Themes menu closes on repeated click)
- i18n: pluralization for time_*_ago functions (fixed "{plural}" appearing in session list)

### Performance
- Git status: optimized from 6 to 2 git process spawns per call (66% reduction)
- RenamePattern::apply(): reduced allocations using Vec<&str> instead of Vec<String>

### Refactoring
- Extracted CursorNavigation trait for modal cursor navigation (DRY, -70 LOC)
- Extracted git_command helpers to reduce duplication
- DRY improvements in system-monitor and status_bar
- Removed dead code across multiple crates (~300 LOC)

## [0.6.0] - 2025-12-16

### Added
- File search modal (Ctrl+Shift+P) with async glob pattern matching
- Content search modal (Ctrl+Shift+F) with regex support for searching text in files
- Theme selection UI with nested dropdown menu in Preferences
  - Color preview for each theme showing bg/fg and accent colors
  - Immediate theme application and config persistence
- Theme refactoring: `all_theme_names()` and `get_by_name()` API for theme system

### Fixed
- Syntax highlighting fallback to theme foreground color for light themes
- Session modal: current session now highlighted with accent color and selectable
- Session modal and file search UX improvements

### Documentation
- Added file search (Ctrl+Shift+P) and content search (Ctrl+Shift+F) shortcuts
- Updated theme documentation with new menu-based selection method

## [0.5.4] - 2025-12-15

### Added
- Session switching modal (Alt+M → Sessions) to switch between open termide sessions
- Static loading indicator for file watcher initialization (replaced animated Braille spinner)

### Fixed
- Word wrap cursor positioning unified calculation
- Mouse selection offset with word wrap enabled
- Disk space indicator formatting (removed space before unit: "411/467GB")

### Performance
- File watcher now uses `ignore` crate to skip .gitignored directories

### Documentation
- Updated project structure in README and architecture docs to reflect workspace crates
- Updated i18n section to show all 9 supported languages
- Added Ctrl+D (duplicate line) to editor help files

## [0.5.3] - 2025-12-15

### Fixed
- Git status color flickering on directory reload (preserve cache during refresh)
- `.git` directory date not updating on commit when viewing repo root
- Single mouse click copying character to clipboard in terminal (now only drag selection copies)
- File manager git status update timing and CPU format display in menu bar

### Changed
- Unified git and filesystem watchers into single `UnifiedWatcher` with separate debounce intervals (300ms files, 1000ms git)

### Performance
- Optimized rendering and reduced memory allocations

## [0.5.2] - 2025-12-14

### Fixed
- Terminal scroll not working during user input wait (scroll events now bypass modal check)
- i18n placeholders showing '{}' instead of values (all 9 language files updated with named placeholders)

### Changed
- Removed duplicate `/i18n/` folder from project root (was unused copy)
- Moved `/help/` files to `crates/panel-misc/help/` for better code organization

## [0.5.1] - 2025-12-13

### Changed
- Terminal panel performance optimizations
  - Use `has_pending_output()` in event loop for efficient redraw triggering
  - Pre-allocate spans Vec for reduced allocations
  - Use O(1) VecDeque methods for row 0 operations
- Remove crates.io publishing (distribution via GitHub Releases, deb/rpm only)

### Fixed
- Git status display in file info modal now shows actual change count
- File manager cursor resets to position 0 when entering subdirectory
- Version consistency across modal, panel-editor, ui-render crates

## [0.5.0] - 2025-12-13

### Added
- Editor Tab key insertion and block indent/unindent (Tab/Shift+Tab)
- External file change detection with reload/close dialog
- Log viewer panel with Editor-based rendering (cursor, selection support)
- Mouse scroll based on cursor position, not panel focus
- Recursive file watching for git repositories

### Changed
- **Major architecture refactoring**: extracted 31 workspace crates from monolithic src/
  - Modular crate structure: app, app-core, app-event, app-modal, app-panel, etc.
  - Improved build times with incremental compilation
  - Better code organization and separation of concerns
- Type-safe panel communication architecture
  - `PanelCommand` enum replacing unsafe `dyn Any` downcasting
  - `CommandResult` enum for type-safe command responses
  - `handle_command()` method on Panel trait
- Modal dialog code consolidation
  - Shared frame rendering with [X] close button
  - Shared input field rendering with cursor
  - Reduced code duplication in search/replace modals
- Config restructured to nested TOML sections
- Async git diff computation to prevent UI freeze
- Gitignore-aware filesystem updates

### Fixed
- File manager cursor position preserved after file deletion
- Copy/paste operations no longer cause unwanted scroll
- Local timezone display for file modification times
- File sizes rounded to whole units and right-aligned
- Git diff deduplication to prevent UI freeze
- FS watcher feedback loops and deleted files display

### Performance
- Async git diff and gitignore-aware FS updates
- Cache `find_repo_root()` and throttle spinner
- Conditional redraw to reduce idle CPU usage
- Memory optimization: removed redundant clones

### Tests
- PanelCommand and CommandResult unit tests (7 tests in core)
- Editor handle_command integration tests (9 tests)
- FileManager handle_command integration tests (8 tests)
- Modal base module tests (4 tests)
- Large file handling tests (7 tests)
  - 10K+ line file loading and navigation
  - Scroll performance benchmark (50K lines in <100ms)

### Code Quality
- Zero TODO/FIXME comments in production code
- All 5 unsafe blocks documented with SAFETY comments
- Replaced 21+ critical `unwrap()` calls with `expect()` + context messages
- Fixed error swallowing in Editor `handle_key()` - errors now shown to user
- Removed dead code: unused `RequestDirSize` event and `PanelProvider` trait methods
- Optimized `get_selected_text()` - no longer copies entire buffer for large selections

## [0.4.0] - 2025-12-07

### Added
- Double-click word selection in editor
  - Select word between nearest delimiters with double-click
  - Proper handling of alphanumeric word boundaries
- Smart word wrapping with word boundary detection
  - Breaks lines at word boundaries when possible
  - Falls back to hard break for words wider than viewport
- Visual line navigation for word-wrapped text
  - Cursor Up/Down moves through visual lines, not buffer lines
  - Preserves preferred column across visual line movements
- Proper Unicode rendering for CJK and combining characters
  - Chinese/Japanese/Korean characters display with correct 2-column width
  - Hindi and other scripts with combining characters render correctly
  - Uses grapheme clusters for proper text segmentation
- Localization for 7 new languages
  - German (de), Spanish (es), French (fr), Hindi (hi)
  - Japanese (ja), Korean (ko), Portuguese (pt), Thai (th), Chinese (zh)
- Panel reordering hotkeys
  - Alt+[ and Alt+] to reorder panels within current group
  - Alt+PageUp/PageDown context-aware (reorder or switch)
- Kitty keyboard protocol support for proper Alt+Cyrillic handling

### Changed
- Major editor architecture decomposition
  - Extracted cursor movement to dedicated modules (physical, visual)
  - Separated rendering into focused modules (line, wrap, cursor, highlights)
  - Created RenderContext for shared rendering state
  - Keyboard handling with Command Pattern
- Translations migrated from Rust code to TOML files
  - Easier to add/update translations
  - Cleaner separation of concerns
- Terminal VT100 parser extracted to dedicated module
- Extensive code cleanup and DRY refactoring
  - Extracted TextInputHandler for modal inputs
  - Created path_utils module for path resolution
  - Added panel downcast helpers (PanelExt trait)

### Fixed
- Cursor Up/Down navigation with word wrap
- Git status tracking in subdirectories
- Editor navigation and viewport issues with word wrap
- App initialization with correct terminal size

### Performance
- Critical hot path optimizations (100-270x faster)
  - Vec pre-allocation when size is known
  - Eliminated unnecessary string allocations
  - Optimized terminal character shift with copy_within

## [0.3.0] - 2025-12-04

### Added
- Release management Claude Code skill for automated release workflow
  - Pre-release quality checks (fmt, clippy, test, build)
  - Multi-source change analysis (uncommitted, commits, file states)
  - Interactive version selection with validation
  - Automated version updates across 12+ files
  - Auto-generated CHANGELOG entries from git history
  - Post-update quality verification
  - Conventional commit generation and git tag creation
- XDG Base Directory Specification support
  - Config: `~/.config/termide/` (or `$XDG_CONFIG_HOME/termide/`)
  - Data: `~/.local/share/termide/` (or `$XDG_DATA_HOME/termide/`)
  - Cache: `~/.cache/termide/` (or `$XDG_CACHE_HOME/termide/`)
  - Proper cross-platform paths (Linux, macOS, Windows)
- Automatic session persistence with configurable retention
  - Sessions save automatically on focus loss (debounced)
  - Auto-cleanup of sessions older than configured retention period (default: 30 days)
  - Per-project session storage
  - Unsaved buffer persistence across sessions
- Comprehensive project documentation
  - CHANGELOG.md with full version history (0.1.0 to 0.2.0)
  - CONTRIBUTING.md with development guidelines
  - Updated issue templates
  - Revised security policy
  - Contributor Covenant Code of Conduct

### Changed
- **BREAKING**: FileManager is now a regular closable panel
  - Removed special fixed left panel handling
  - FileManager can be closed, resized, and moved between groups
  - Default initialization with 2 FileManager panels (50/50 layout)
  - Simplified architecture (-350 lines of code)
  - All panels are now first-class citizens with identical capabilities
  - Existing sessions will load with default layout

## [0.2.0] - 2025-12-04

### Added
- Comprehensive logging system with configurable levels (debug, info, warn, error)
- Real-time git diff visualization in editor line numbers
  - Display uncommitted changes with color-coded line numbers
  - Show deletion markers with count on horizontal lines
  - In-memory diff computation with debounced updates (300ms)
  - Localized deletion marker text (English/Russian)
- Configurable word wrap option in editor (enabled by default)
- Per-project session storage with automatic unsaved buffer persistence
- Automatic cleanup of old sessions (configurable retention period, default 30 days)
- New configuration options:
  - `word_wrap` - Enable/disable word wrap in editor
  - `min_log_level` - Minimum logging level (debug, info, warn, error)
  - `session_retention_days` - Session cleanup retention period
  - `show_git_diff` - Toggle git diff visualization
  - `fm_extended_view_width` - Minimum width for file manager extended view

### Changed
- Rewrite clipboard system using arboard library
  - Support both CLIPBOARD and PRIMARY selections on Linux
  - More reliable cross-platform clipboard operations
- Improve cursor rendering across all panels
  - Use inverse colors instead of theme selection colors
  - Better visual contrast and cursor visibility
  - Handle reverse attribute correctly in terminal panels
- Improve config auto-completion with hash-based detection
  - Automatically detect ALL missing config keys
  - No manual maintenance of required_keys array
  - Correctly handle optional fields
  - Normalize config file format

### Fixed
- Word wrap rendering bug causing single-line display
- Cursor going off-screen with git diff deletion markers
- Viewport calculations now account for virtual lines (deletion markers)
- Empty line rendering in word wrap mode
- Modified flag after undo/redo operations
- File manager navigation now remembers directory when going up
- Old API tests marked as ignored to fix CI

## [0.1.5] - 2025-12-02

### Fixed
- Package build issues for .deb and .rpm on ARM64 architecture
  - Removed aarch64 from package build matrices to avoid cross-compilation issues
  - Binary tarballs still support all platforms including ARM64

### Added
- Local package build testing script (`scripts/test-packages.sh`)

## [0.1.4] - 2025-12-02

### Added
- Package manager distribution support:
  - Debian/Ubuntu packages (.deb)
  - Fedora/RHEL/CentOS packages (.rpm)
  - Arch Linux AUR packages (source and binary variants)
  - Homebrew formula for macOS/Linux
- Enhanced Nix Flake support:
  - Add `packages.default` output with `rustPlatform.buildRustPackage`
  - Add `apps.default` for `nix run` support
  - Add `overlays.default` for nixpkgs integration
  - Install help files and themes in postInstall phase
- Automatic config file completion for new configuration keys
- Comprehensive installation documentation for all package managers

### Changed
- Reorganize README installation section with collapsible details blocks
- Use emoji icons for better visual navigation in documentation
- Update GitHub Actions workflow to build .deb and .rpm packages automatically

### Fixed
- GitHub Actions workflow dependencies (crates.io publication now requires all builds to succeed)
- Binary paths in cargo-deb and cargo-generate-rpm for cross-compilation
- Fail-fast strategy in build matrices to see all failures

## [0.1.3] - 2025-12-02

### Added
- **Accordion panel system** - Major architectural improvement
  - Smart panel stacking based on terminal width
  - Vertical accordion layout within horizontal groups
  - One expanded panel per group, others collapse to title bar
  - Configurable minimum panel width threshold (80 characters)
- New navigation hotkeys:
  - `Alt+Up/Down` - Navigate panels within group
  - `Alt+PgUp/PgDn` - Move panel to previous/next group
  - `Alt+Home/End` - Move panel to first/last group
  - `Alt+Plus/Minus` - Increase/decrease active group width
  - `Alt+Backspace` - Toggle panel stacking (merge/unstack)
- Developer documentation (`doc/en/architecture.md`, `doc/en/developer-guide.md`)

### Changed
- Complete panel layout system redesign
  - New `LayoutManager` for centralized panel group management
  - New `PanelGroup` for vertical panel stacking
  - Separate panel rendering logic
- Panel width management improvements
  - Fix redistribute after group deletion (8 locations)
  - Add zero-sum balance correction for resize operations
  - Fix auto-stacking calculation to use average width
  - Add proportional width redistribution across all groups

### Removed
- **BREAKING**: Removed `LayoutMode` (SimplePanel/MultiPanel) in favor of dynamic groups
- **BREAKING**: Changed panel navigation model from flat to hierarchical (groups + panels)

## [0.1.2] - 2025-11-29

### Added
- Duplicate line/selection feature (`Ctrl+D`)
- Replace operation feedback ("Replaced N occurrence(s)" message)
- File size validation before opening in editor (100 MB limit)
- Configurable tab size support (reads from `config.toml`)
- Crates.io publishing in release workflow

### Changed
- Migrate to semantic versioning tags without 'v' prefix (e.g., `0.1.2` instead of `v0.1.2`)
- Update license in Cargo.toml from dual (MIT OR Apache-2.0) to MIT only
- Improve error handling across the application
  - Replace panic with graceful error handling in theme parsing
  - Falls back to hardcoded default theme on parse errors
  - Better mutex error handling in terminal (22 `.lock().unwrap()` replaced with `.expect()`)
- Add clear error messages for oversized files

### Fixed
- Application crashes from invalid theme files
- Editor tab size now respects user configuration

## [0.1.1] - 2025-11-25

### Added
- Interactive search modal (`Ctrl+F`) with live preview and match counter
- Interactive replace modal (`Ctrl+H`) with dual input fields
- Tab/Shift+Tab navigation in search mode
- State preservation for search/replace queries
- `[X]` close button on editor panels
- Arrow key navigation between fields in replace modal

### Fixed
- Replace operation skipping matches on same line
- Inconsistent cursor positioning (now always at end of match with selection)
- Prev/Next navigation buttons resetting search state
- Escape key behavior (closes search first, then panel)
- Enter key in replace modal (now replaces instead of deleting)

### Changed
- Update match positions after replacement operations
- Standardize cursor positioning across all search/replace operations

## [0.1.0] - 2025-11-25

### Added
- Initial TermIDE release with complete feature set
- Terminal-based IDE with syntax highlighting for 15+ programming languages
  - Rust, Python, JavaScript, TypeScript, Go, C/C++, Java, Ruby, PHP
  - Haskell, Nix, HTML, CSS, JSON, TOML, YAML, Bash, Markdown
- Smart file manager with intuitive TUI interface
  - File type icons with attributes column
  - Symlink and executable file detection
  - Advanced keyboard and mouse selection controls
  - Recursive git status for directories
- Integrated virtual terminal with full PTY support
  - Ctrl+Shift+V paste with bracketed paste mode
  - 24 FPS rendering
  - Scrollback buffer and ANSI color support
- Multi-panel layout system
- Git integration
  - Background git status monitoring with file watching
  - Automatic updates on repository changes
  - Color-coded status indicators
  - Dimmed styling for gitignored files
  - Support for repository subdirectories
- 12 built-in themes
  - Dark: Default, Midnight, Dracula, OneDark, Monokai, Nord, Solarized Dark
  - Light: Atom One Light, Ayu Light, GitHub Light, Material Lighter, Solarized Light
  - Custom theme support from config directory
- System resource monitoring
  - Real-time CPU and RAM usage
  - Color-coded alerts
- Multi-language support (English, Russian)
  - Full Cyrillic keyboard layout support
  - Case-preserving hotkey translation
- Mouse support for all panels and UI elements
- Clipboard system for cut/copy/paste operations
- Batch file operations (copy, move, delete)
- Quit confirmation for unsaved changes and running processes
- Robust error handling and file size limits
- Automatic directory refresh on filesystem changes
- Cross-platform support (Linux, macOS, Windows via WSL)
- Multi-architecture builds (x86_64, ARM64)

### Technical
- Built with Rust using ratatui TUI framework
- Crossterm for cross-platform terminal manipulation
- Portable-pty for PTY implementation
- Tree-sitter for syntax highlighting
- Ropey for text buffer management
- Sysinfo for system resource monitoring
- Pre-commit hooks for code quality
- Comprehensive test suite
