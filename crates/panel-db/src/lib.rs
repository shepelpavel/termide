//! Database viewer panel for termide.
//!
//! A read-only, git-status-shaped panel: a table selector on top and a 2D
//! pseudographic grid below. Connections come from bookmarks (the `path` field
//! holds a DB URL). Queries run on `termide-db`'s background runtime; the panel
//! polls the result receivers from `tick()`, so the UI never blocks.
//!
//! Scope of this first cut: connect → list tables → browse a table with a
//! cell cursor, sliding-window pagination, single-column sort, and copy.
//! Filtering, the row-detail modal, schema selectors and the in-app password
//! prompt are layered on next (see `ROADMAP.md.tmp`).

mod actions;
mod render;

use std::any::Any;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use termide_config::Config;
use termide_core::{
    CommandResult, KeyChord, Panel, PanelCommand, PanelEvent, RenderContext, ThemeColors,
    WidthPreference,
};
use termide_db::{
    ColumnInfo, Condition, DbBackend, DbConnection, DbError, Page, PageRequest, SortDir,
};
use termide_modal::ActiveModal;
use termide_state::PendingAction;

/// Default sliding-window size (rows held in memory per page fetch).
const WINDOW: u64 = 200;

/// Which zone has focus inside the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    TableSelector,
    Grid,
}

/// Connection lifecycle. Connecting happens on a throwaway thread (the
/// `DbConnection::connect` call blocks), so the UI stays responsive.
enum ConnState {
    Connecting(Receiver<Result<DbConnection, DbError>>),
    Connected(DbConnection),
    Failed(String),
}

/// The database viewer panel.
pub struct DbPanel {
    /// Full connection URL (may carry a password — never rendered verbatim).
    url: String,
    /// Display label (bookmark description or sanitized URL).
    label: String,
    backend: DbBackend,
    conn: ConnState,

    // --- catalog ---
    tables: Vec<String>,
    selected_table: Option<String>,
    columns: Vec<ColumnInfo>,

    // --- current page (sliding window) ---
    page: Page,
    total_rows: Option<i64>,
    offset: u64,

    // --- grid cursor / scroll ---
    cursor_row: usize,
    cursor_col: usize,
    row_scroll: usize,
    col_scroll: usize,

    // --- query state ---
    filters: Vec<Condition>,
    order_by: Vec<(String, SortDir)>,

    // --- focus / selector ---
    section: Section,
    table_dropdown_open: bool,
    dropdown_cursor: usize,
    dropdown_scroll: usize,

    // --- async receivers (polled in tick) ---
    tables_rx: Option<Receiver<Result<Vec<String>, DbError>>>,
    columns_rx: Option<Receiver<Result<Vec<ColumnInfo>, DbError>>>,
    count_rx: Option<Receiver<Result<i64, DbError>>>,
    page_rx: Option<Receiver<Result<Page, DbError>>>,
    loading: bool,

    // --- render cache ---
    cached_theme: ThemeColors,
    last_area: Rect,
    /// Number of data rows visible in the grid viewport (set during render).
    visible_rows: usize,

    /// Pending modal request, polled by the app via `take_modal_request`.
    modal_request: Option<(PendingAction, ActiveModal)>,
}

impl DbPanel {
    /// Open a panel for `url`. `label` is the bookmark description (falls back to
    /// a sanitized URL). Connection starts immediately in the background.
    pub fn new(url: impl Into<String>, label: impl Into<String>) -> Self {
        let url = url.into();
        let label_in = label.into();
        let backend = DbBackend::from_url(&url).unwrap_or(DbBackend::Sqlite);
        let label = if label_in.is_empty() {
            sanitize_url(&url)
        } else {
            label_in
        };
        let conn = spawn_connect(url.clone());
        Self {
            url,
            label,
            backend,
            conn: ConnState::Connecting(conn),
            tables: Vec::new(),
            selected_table: None,
            columns: Vec::new(),
            page: Page::default(),
            total_rows: None,
            offset: 0,
            cursor_row: 0,
            cursor_col: 0,
            row_scroll: 0,
            col_scroll: 0,
            filters: Vec::new(),
            order_by: Vec::new(),
            section: Section::TableSelector,
            table_dropdown_open: false,
            dropdown_cursor: 0,
            dropdown_scroll: 0,
            tables_rx: None,
            columns_rx: None,
            count_rx: None,
            page_rx: None,
            loading: true,
            cached_theme: ThemeColors::default(),
            last_area: Rect::default(),
            visible_rows: 0,
            modal_request: None,
        }
    }

    /// The connection URL (used for session persistence / reconnect).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Take a pending modal request (polled by the app each frame).
    pub fn take_modal_request(&mut self) -> Option<(PendingAction, ActiveModal)> {
        self.modal_request.take()
    }

    /// Build the shared-status-bar summary for the current view.
    fn status_text(&self) -> String {
        match &self.conn {
            ConnState::Connecting(_) => format!("{}: connecting…", self.label),
            ConnState::Failed(e) => format!("{}: {}", self.label, e),
            ConnState::Connected(_) => {
                let Some(table) = &self.selected_table else {
                    return format!("{} · {} · select a table", self.label, self.backend.label());
                };
                let n = self.page.rows.len() as u64;
                let range = if n == 0 {
                    "0 rows".to_string()
                } else {
                    format!("rows {}–{}", self.offset + 1, self.offset + n)
                };
                let total = match self.total_rows {
                    Some(t) => format!(" of {t}"),
                    None => " of …".to_string(),
                };
                let sort = match self.order_by.first() {
                    Some((c, d)) => {
                        let arrow = if *d == SortDir::Asc { "↑" } else { "↓" };
                        format!(" · sort: {c} {arrow}")
                    }
                    None => String::new(),
                };
                let filter = if self.filters.is_empty() {
                    String::new()
                } else {
                    format!(" · filter: {}", self.filters.len())
                };
                format!(
                    "{} · {} · {}{}{}{}",
                    self.label, table, range, total, sort, filter
                )
            }
        }
    }

    fn status_event(&self) -> PanelEvent {
        PanelEvent::SetStatusMessage {
            message: self.status_text(),
            is_error: matches!(self.conn, ConnState::Failed(_)),
        }
    }

    /// (Re)issue columns + count + page queries for the selected table.
    fn reload_table(&mut self) {
        self.offset = 0;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.row_scroll = 0;
        self.col_scroll = 0;
        self.total_rows = None;
        self.filters.clear();
        self.order_by.clear();
        self.reload_all();
    }

    /// Re-issue all three queries (columns, count, page) for the current
    /// table/filter/sort/offset.
    fn reload_all(&mut self) {
        let Some(table) = self.selected_table.clone() else {
            return;
        };
        let order_by = self.order_by.clone();
        let filters = self.filters.clone();
        let offset = self.offset;
        let rxs = if let ConnState::Connected(conn) = &self.conn {
            Some((
                conn.columns(table.clone()),
                conn.count(table.clone(), filters.clone()),
                conn.page(PageRequest {
                    table,
                    filters,
                    order_by,
                    limit: WINDOW,
                    offset,
                }),
            ))
        } else {
            None
        };
        if let Some((c, n, p)) = rxs {
            self.columns_rx = Some(c);
            self.count_rx = Some(n);
            self.page_rx = Some(p);
            self.loading = true;
        }
    }

    /// Re-issue only the page query (window move / sort change), keeping the
    /// known column list and total count.
    fn reload_page(&mut self) {
        let Some(table) = self.selected_table.clone() else {
            return;
        };
        let order_by = self.order_by.clone();
        let filters = self.filters.clone();
        let offset = self.offset;
        let rx = if let ConnState::Connected(conn) = &self.conn {
            Some(conn.page(PageRequest {
                table,
                filters,
                order_by,
                limit: WINDOW,
                offset,
            }))
        } else {
            None
        };
        if let Some(p) = rx {
            self.page_rx = Some(p);
            self.loading = true;
        }
    }

    /// Poll all in-flight receivers; returns true if anything changed.
    fn poll_async(&mut self) -> bool {
        let mut changed = false;

        // Connection establishment.
        if let ConnState::Connecting(rx) = &self.conn {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(conn) => {
                        self.conn = ConnState::Connected(conn);
                        if let ConnState::Connected(c) = &self.conn {
                            self.tables_rx = Some(c.list_tables());
                        }
                    }
                    Err(e) => {
                        let msg = if e.is_auth() {
                            format!("auth failed — add a password to the bookmark URL ({e})")
                        } else {
                            e.to_string()
                        };
                        self.conn = ConnState::Failed(msg);
                        self.loading = false;
                    }
                }
                changed = true;
            }
        }

        if let Some(rx) = &self.tables_rx {
            if let Ok(result) = rx.try_recv() {
                self.tables_rx = None;
                match result {
                    Ok(tables) => {
                        self.tables = tables;
                        if self.selected_table.is_none() {
                            if let Some(first) = self.tables.first().cloned() {
                                self.selected_table = Some(first);
                                self.section = Section::Grid;
                                self.reload_table();
                            } else {
                                self.loading = false;
                            }
                        }
                    }
                    Err(e) => self.conn = ConnState::Failed(e.to_string()),
                }
                changed = true;
            }
        }

        if let Some(rx) = &self.columns_rx {
            if let Ok(result) = rx.try_recv() {
                self.columns_rx = None;
                if let Ok(cols) = result {
                    self.columns = cols;
                }
                changed = true;
            }
        }

        if let Some(rx) = &self.count_rx {
            if let Ok(result) = rx.try_recv() {
                self.count_rx = None;
                if let Ok(n) = result {
                    self.total_rows = Some(n);
                }
                changed = true;
            }
        }

        if let Some(rx) = &self.page_rx {
            if let Ok(result) = rx.try_recv() {
                self.page_rx = None;
                self.loading = false;
                match result {
                    Ok(page) => {
                        self.page = page;
                        self.clamp_cursor();
                    }
                    Err(e) => {
                        self.conn = ConnState::Failed(e.to_string());
                    }
                }
                changed = true;
            }
        }

        changed
    }

    fn clamp_cursor(&mut self) {
        let rows = self.page.rows.len();
        if rows == 0 {
            self.cursor_row = 0;
        } else if self.cursor_row >= rows {
            self.cursor_row = rows - 1;
        }
        let cols = self.col_count();
        if cols == 0 {
            self.cursor_col = 0;
        } else if self.cursor_col >= cols {
            self.cursor_col = cols - 1;
        }
    }

    /// Number of columns to render (from catalog, falling back to page columns).
    fn col_count(&self) -> usize {
        if !self.columns.is_empty() {
            self.columns.len()
        } else {
            self.page.columns.len()
        }
    }

    fn column_names(&self) -> Vec<String> {
        if !self.columns.is_empty() {
            self.columns.iter().map(|c| c.name.clone()).collect()
        } else {
            self.page.columns.clone()
        }
    }

    fn is_connected(&self) -> bool {
        matches!(self.conn, ConnState::Connected(_))
    }
}

/// Spawn a thread that connects and ships the handle (or error) back.
fn spawn_connect(url: String) -> Receiver<Result<DbConnection, DbError>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("termide-db-connect".into())
        .spawn(move || {
            let _ = tx.send(DbConnection::connect(&url));
        })
        .ok();
    rx
}

/// Strip a password from a URL for display (`scheme://user:***@host/…`).
fn sanitize_url(url: &str) -> String {
    // Find "://", then the authority up to the next '/'.
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after = scheme_end + 3;
    let rest = &url[after..];
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if let Some(at) = authority.rfind('@') {
        let userinfo = &authority[..at];
        if let Some(colon) = userinfo.find(':') {
            let user = &userinfo[..colon];
            return format!(
                "{}://{}:***@{}{}",
                &url[..scheme_end],
                user,
                &authority[at + 1..],
                &rest[authority_end..]
            );
        }
    }
    url.to_string()
}

impl Panel for DbPanel {
    fn name(&self) -> &'static str {
        "db"
    }

    fn title(&self) -> String {
        match &self.selected_table {
            Some(t) => format!("DB: {} · {}", self.label, t),
            None => format!("DB: {}", self.label),
        }
    }

    fn prepare_render(&mut self, theme: &termide_theme::Theme, _config: &Arc<Config>) {
        self.cached_theme = ThemeColors::from(theme);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, ctx: &RenderContext) {
        self.last_area = area;
        self.render_content(area, buf, ctx.is_focused);
    }

    fn handle_key(&mut self, chord: KeyChord) -> Vec<PanelEvent> {
        self.handle_key_impl(chord)
    }

    fn tick(&mut self) -> Vec<PanelEvent> {
        if self.poll_async() {
            vec![PanelEvent::NeedsRedraw, self.status_event()]
        } else {
            vec![]
        }
    }

    fn handle_command(&mut self, _cmd: PanelCommand<'_>) -> CommandResult {
        CommandResult::None
    }

    fn captures_escape(&self) -> bool {
        self.table_dropdown_open
    }

    fn width_preference(&self) -> WidthPreference {
        WidthPreference::PreferWide
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
