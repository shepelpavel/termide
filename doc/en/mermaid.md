# Mermaid Diagram Viewer

TermIDE renders [Mermaid](https://mermaid.js.org/) source files (`.mmd`,
`.mermaid`) as a read-only diagram drawn in text pseudographics, instead of
opening the raw source. The same renderer draws embedded ```` ```mermaid ````
blocks in the [Markdown preview](markdown.md).

## Opening

- **`F3`** on a `.mmd` / `.mermaid` file — open the rendered diagram.
- **`Enter`** or **`F4`** — open the raw source in the editor (as for any text
  file).

## Diagram ↔ Source

The diagram and the source editor are two views of the same file. Switch between
them in place — the panel is replaced, not stacked:

- **`Ctrl+E`** (configurable via `[viewer.keybindings] toggle_view`).
- The clickable **`Edit`** chip in the status bar.

Switching back to the diagram is blocked while the source has unsaved changes —
save first.

## Supported diagram types

Parsed and laid out without an external Mermaid engine (pure Rust):

- **flowchart** / **graph** — layered (Sugiyama-style) layout with orthogonal
  elbow edges, box-drawing junctions, and edge labels.
- **sequenceDiagram** — participant boxes, lifelines, arrows (solid/dashed/open/
  cross heads), notes, and self-messages.
- **stateDiagram** — reuses the flowchart engine (`[*]` becomes start/end nodes).
- **classDiagram** / **erDiagram** — boxes with members/attributes drawn in
  compartments.
- **gantt** — timeline bars grouped by section, with a gridded table layout: a
  date axis duplicated above and below, vertical gridlines, and `┼` dividers
  between sections.
- **pie**, **journey**, **mindmap**, **timeline**, **gitGraph**, **quadrant**.

Diagram kinds that are not yet laid out (e.g. `requirementDiagram`,
`C4Context`) show the source with an informative note.

## Navigation and copy

- `↑`/`↓`/`←`/`→` (or `k`/`j`/`h`/`l`) — scroll the canvas in two dimensions.
- `PageUp`/`PageDown` (or `Space`) — page vertically; `Home`/`End` — top /
  bottom; the mouse wheel and a right-edge scrollbar scroll too.
- **`y`** or **`Ctrl+C`**, or the **`Copy diagram`** entry in the panel `[≡]`
  menu — copy the rendered diagram (the pseudographics) to the clipboard.

The panel persists across sessions and reopens at the same file.
