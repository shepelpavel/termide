# Database Viewer

TermIDE includes a built-in, **read-only** database browser for **SQLite**,
**PostgreSQL** and **MySQL/MariaDB**. It connects over the native wire protocol
(no external client tools required), lists a database's tables, and shows rows
in a scrollable grid with sorting and per-column filtering.

The viewer never writes: it only issues `SELECT` and catalog queries.

## Opening a connection

Connections are ordinary **bookmarks** whose path is a database URL. Add a
bookmark (Bookmarks menu → Add) and put the connection URL in the *Path* field:

| Engine | URL example |
|--------|-------------|
| SQLite | `sqlite:///absolute/path/to/app.db` |
| PostgreSQL | `postgres://user@host:5432/mydb` |
| MySQL / MariaDB | `mysql://user@host:3306/mydb` |

Activating a database bookmark opens the Database panel and connects in the
background — the UI never blocks while connecting.

### Passwords

As with SFTP/FTP bookmarks, the password is **not** treated specially: the
bookmark stores exactly the URL you enter. You can either rely on a
password-less authentication path (PostgreSQL `~/.pgpass`/peer/trust, a Unix
socket, `PGPASSWORD`, …) or include the password directly in the URL
(`postgres://user:secret@host/db`). If you embed it, note that the bookmark is
saved verbatim to `bookmarks.toml`, so treat that file accordingly.

## Layout

```
┌─ Database ──────────────────────────────────────────┐
│ Table ▾ users                                        │
│ id │ name  │ score │ active                          │
│  1 │ alice │ 1.5   │ true                             │
│  2 │ bob   │ 2.25  │ false                            │
│  3 │ carol │ NULL  │ true                             │
└──────────────────────────────────────────────────────┘
```

A **table selector** sits on top; the **data grid** fills the rest. When the
connection URL omits a database (PostgreSQL/MySQL), a **database selector**
appears to its left — pick a database and the viewer reconnects to it. The grid
has a 2D cell cursor: the highlighted cell is the target for sorting, filtering
and copying. The shared status bar shows the current range, total row count,
active sort and filter, e.g. `app.db · users · rows 1–200 of 1203 · sort: name ↑ · filter: 1`.

## Keys

| Key | Action |
|-----|--------|
| `Tab` | Switch focus between the table selector and the grid |
| `Enter` / `Space` (on selector) | Open the table dropdown |
| `↑ ↓ ← →` | Move the cell cursor (auto-scrolls rows and columns) |
| `PageUp` / `PageDown` | Move a screenful; loads the next/previous window at the edges |
| `Home` / `End` | Jump to the first / last row |
| `S` | Sort by the current column — cycles ascending → descending → unsorted |
| `Ctrl+F` / `F3` | Filter the current column (opens the filter dialog) |
| `Alt+F` | Clear all filters |
| `Space` / `F12` (on grid) | Show the full current row (key/value), with copy options |
| `Ctrl+C` | Copy the current cell value |
| `Ctrl+Y` | Copy the current row as tab-separated values |
| `Ctrl+R` | Refresh the table list and reload the current view |

These action keys are configurable under `[database.keybindings]` in the config
and appear in the Help panel. Navigation keys (Tab, arrows, PageUp/Down,
Home/End) are fixed.

**Mouse:** click the table selector to open it, click a column header to cycle
its sort, and click a data cell to move the cursor there.

## Sorting

Press `s` on a column to sort by it server-side. The cycle is
**ascending → descending → unsorted**; "unsorted" issues no `ORDER BY`, which is
the cheapest option on large tables. The sorted column shows a `↑`/`↓` arrow in
its header. One column at a time.

> On PostgreSQL, paging through an *unsorted* table can return rows in a
> slightly different order between pages (the engine's natural order isn't
> stable across `OFFSET`s). Apply a sort if you need a stable order.

## Filtering

`Ctrl+F` (or `F3`) opens the filter dialog, which lists **every column** — one
row each with the column name, an operator (cycle with ←/→; the first option
means "no condition") and a value typed in place. The available operators depend
on the column's type:

- **Text:** contains, starts with, ends with, =, ≠, is null, is not null
- **Numeric / date:** =, ≠, >, ≥, <, ≤, is null, is not null
- **Boolean:** =, ≠, is null, is not null

Bottom buttons: **Apply**, **Clear filters**, **Cancel**. Applying replaces the
whole filter set; conditions on different columns combine with **AND**.
`contains`/`starts with`/`ends with` are case-insensitive. `Alt+F` clears all
filters without opening the dialog. Values are always sent as bound parameters.

## Row detail

Press `Space` on a row to open a detail dialog listing every column as
key/value — useful for long text, JSON or wide rows that don't fit the grid. The
dialog can copy the row as **TSV**, **JSON**, or an **INSERT** statement.

## Pagination

Rows are fetched in windows; only the current window is held in memory, so even
very large tables stay responsive. Moving past the end of a window loads the
next one automatically. The total row count (respecting the active filter) is
fetched in the background and appears in the status bar once ready.

## Limitations (current)

- Read-only — no editing, no arbitrary SQL.
- Exotic column types (JSON, arrays, UUID, timestamps, unsigned integers) are
  shown as empty/`NULL` in this version; common scalar types render fully.
- Database selection is supported when the URL omits it; schema selection
  (PostgreSQL) still defaults to the current schema.
- No in-app password prompt yet — use a password-less auth path or include the
  password in the URL (see above).
