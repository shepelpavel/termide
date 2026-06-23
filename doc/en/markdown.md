# Markdown Preview

TermIDE renders Markdown files (`.md`, `.markdown`) as a read-only preview panel
instead of opening the raw source.

## Opening

- **`F3`** on a `.md` file — open the rendered preview.
- **`Enter`** or **`F4`** — open the raw source in the editor (as for any text
  file).

## Preview ↔ Source

The preview and the source editor are two views of the same file. Switch
between them in place — the panel is replaced, not stacked:

- **`Ctrl+E`** (configurable via `[viewer.keybindings] toggle_view`).
- The clickable **`Edit`** chip in the status bar.

In the preview, `Edit: No` means you are viewing the rendered document; clicking
it (or `Ctrl+E`) opens the **editable** source. In the source editor, the same
toggle returns to the preview. Switching back to the preview is blocked while
the source has unsaved changes — save first.

## What is rendered

Parsed with `pulldown-cmark` and drawn as text pseudographics:

- Headings (prefixed with `#` markers, accent colour, bold).
- Bold, italic, strikethrough, and inline `code`.
- Bulleted and ordered lists, including nesting.
- Block quotes, prefixed with `│`.
- Fenced code blocks, syntax-highlighted with the same engine as the editor.
- Tables, drawn with box-drawing borders.
- Horizontal rules and links (underlined, clickable).
- Images as a clickable `🖼` pictogram followed by the alt text (no terminal
  graphics protocol).

## Navigation, selection, links

The preview has a movable cursor and supports text selection:

- `↑`/`↓`/`←`/`→` (or `k`/`j`/`h`/`l`) — move the cursor.
- `PageUp`/`PageDown` (or `Space`) — page up/down; `Home`/`End` — line ends;
  `g`/`G` — document start/end.
- Hold **`Shift`** with movement, or **drag with the mouse**, to select text.
- **`Ctrl+C`** copies the selection (or the cursor's line when nothing is
  selected) to the clipboard.
- Mouse wheel scrolls.
- **Click a link** (or press `Enter` with the cursor on it) to open it in the
  browser; image pictograms open the image URL the same way.

The panel persists across sessions and reopens at the same file.
