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
- **`Ctrl+G`** prompts for a path **or `http(s)://` URL** and opens it in the
  matching viewer (HTML, Markdown, image, or text) — a quick jump to a sibling
  file, or a basic text-mode browse of a web page (see *Fetching URLs* below).
- Mouse wheel scrolls.
- **Follow a link** — click it, or press `Enter` with the cursor on it. By
  default links open **in the panel**: a fetched (URL-backed) page navigates in
  place (relative links resolve against the page URL), a web link from a
  file-backed view opens in a new viewer, and a link to an **image** opens in
  the image preview. Two settings choose the default destination —
  `[viewer] open_links` for pages and `[viewer] open_images` for image links —
  each `panel` (default) or `external`.
- **`O`** always opens the link under the cursor in the external browser
  (regardless of the setting).
- **`[` / `]`** (or **`Backspace`** for back) step back / forward through the
  page history of a navigated view.

## Fetching URLs

The **Windows ▸ Web** menu item opens a URL prompt directly (a discoverable
entry point); `Ctrl+G` in any viewer does the same. `Ctrl+G` with an
`http(s)://` address fetches the document in the background
and opens it routed by `Content-Type` (HTML → this viewer, Markdown → the
Markdown viewer, other text → shown verbatim). The fetch is deliberately
bounded — this is a reader, not a browser engine:

- `http` and `https` only; TLS is verified (no opt-out).
- 15-second timeout; at most 5 redirects, and an `https` origin is never
  downgraded to `http`.
- Responses over 8 MiB are rejected.
- Embedded resources are **not** loaded — `<img>` stays a `🖼` pictogram, so a
  page cannot phone home through the viewer.

Links inside a fetched page are followed **in place** (`Enter`/click), with
relative links resolved against the page URL and `[`/`]` (or `Backspace`) for
history; `O` opens a link in the real browser instead. URL-loaded views are not
restored across sessions.

A file-backed panel persists across sessions and reopens at the same file.
