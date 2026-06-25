# HTML Preview

TermIDE renders HTML files (`.html`, `.htm`) as a read-only preview panel —
text pseudographics, not the raw source. It is *not* a browser: author CSS and
scripts are ignored, and layout is a fixed tag→style mapping.

## Opening

- **`F3`** on an `.html` file — open the rendered preview.
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

Tokenized with `html5ever` (no DOM, no CSS) and drawn as text pseudographics. A
supported subset of tags maps to styled blocks and inline runs; **unknown tags
are transparent** — their content still renders. HTML entities (`&amp;`,
`&#39;`) are decoded; `<script>`, `<style>`, and document `<head>` content is
dropped.

- Headings `<h1>`–`<h6>` (prefixed with `#` markers, accent colour, bold).
- Paragraphs and block containers (`<p>`, `<div>`, `<section>`, …), separated by
  blank lines.
- Inline emphasis: `<b>`/`<strong>`, `<i>`/`<em>`, `<u>`, `<s>`/`<del>`, and
  `<kbd>`/`<mark>` (reverse video).
- Inline `<code>` and `<pre>` code blocks (kept verbatim).
- Bulleted `<ul>` and ordered `<ol>` lists; `<blockquote>` prefixed with `│`.
- `<table>` (with `<thead>` header rows) drawn with box-drawing borders.
- `<a href>` links (underlined, clickable) and `<img>` as a clickable `🖼`
  pictogram followed by the alt text.
- `<br>` line breaks, `<hr>` rules, and `<details>`/`<summary>` (shown expanded).

The same engine renders HTML embedded inside the [Markdown preview](markdown.md).

## Navigation, selection, links

The preview has a movable cursor and supports text selection:

- `↑`/`↓`/`←`/`→` (or `k`/`j`/`h`/`l`) — move the cursor.
- `PageUp`/`PageDown` (or `Space`) — page up/down; `Home`/`End` — line ends;
  `g`/`G` — document start/end.
- Hold **`Shift`** with movement, or **drag with the mouse**, to select text.
- **`Ctrl+C`** copies the selection (or the cursor's line when nothing is
  selected) to the clipboard.
- **`Ctrl+F`** opens incremental search; **`Ctrl+R`** reloads the file from disk.
- **`Ctrl+G`** prompts for a path and opens it in the matching viewer (HTML,
  Markdown, image, or text) — a quick way to jump to a sibling file.
- Mouse wheel scrolls.
- **Click a link** (or press `Enter` with the cursor on it) to open it in the
  browser; image pictograms open the image URL the same way.

The panel persists across sessions and reopens at the same file.
