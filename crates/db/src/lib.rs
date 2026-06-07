//! Database access core for termide.
//!
//! Engine-agnostic, read-only browsing of SQLite / PostgreSQL / MySQL: connect
//! from a URL, list tables, and fetch paginated rows. Queries run on a
//! background tokio runtime; callers interact through a synchronous handle that
//! polls results over a channel, mirroring the rest of termide's async-to-TUI
//! bridges (VFS, LSP).

mod engine;
mod error;
mod value;

use std::sync::mpsc;
use std::thread::{self, JoinHandle};

pub use error::DbError;
pub use value::DbValue;

/// One decoded result row.
pub type ColumnValueRow = Vec<DbValue>;

/// Which engine a URL/connection targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbBackend {
    Sqlite,
    Postgres,
    MySql,
}

impl DbBackend {
    /// Classify a connection URL by its scheme.
    pub fn from_url(url: &str) -> Result<Self, DbError> {
        let scheme = url.split(':').next().unwrap_or("");
        match scheme {
            "sqlite" => Ok(DbBackend::Sqlite),
            "postgres" | "postgresql" => Ok(DbBackend::Postgres),
            "mysql" | "mariadb" => Ok(DbBackend::MySql),
            other => Err(DbError::UnsupportedScheme(other.to_string())),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DbBackend::Sqlite => "SQLite",
            DbBackend::Postgres => "PostgreSQL",
            DbBackend::MySql => "MySQL",
        }
    }
}

/// Broad type category of a column, used to pick relevant filter operators and
/// to coerce filter input. Exotic types (json/array/uuid/…) fall to `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCategory {
    Number,
    Text,
    Bool,
    Date,
    Bytes,
    Other,
}

/// One column's name and inferred category (from the engine catalog).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnInfo {
    pub name: String,
    pub category: TypeCategory,
}

/// Sort direction for an `ORDER BY` term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// A filter comparison operator. `IsNull`/`IsNotNull` take no value; the LIKE
/// family (`Contains`/`StartsWith`/`EndsWith`) applies to text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Contains,
    StartsWith,
    EndsWith,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    IsNull,
    IsNotNull,
}

/// One `WHERE` condition: `column op value`. `value` is `None` for the null
/// operators. Values are always bound as parameters, never interpolated.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub column: String,
    pub op: FilterOp,
    pub value: Option<DbValue>,
}

/// A request for one page of a table's rows, with optional server-side filtering
/// and sorting. Multiple conditions are combined with `AND`.
#[derive(Debug, Clone, Default)]
pub struct PageRequest {
    pub table: String,
    pub filters: Vec<Condition>,
    pub order_by: Vec<(String, SortDir)>,
    pub limit: u64,
    pub offset: u64,
}

/// A page of rows plus the column names of the result set.
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub columns: Vec<String>,
    pub rows: Vec<ColumnValueRow>,
    /// The offset this page started at (echoes the request).
    pub offset: u64,
    /// Whether at least one more row exists past this page.
    pub has_more: bool,
}

/// What the caller asked the worker to do; each carries a reply channel the
/// caller polls non-blockingly.
enum Request {
    ListTables(mpsc::Sender<Result<Vec<String>, DbError>>),
    Columns {
        table: String,
        reply: mpsc::Sender<Result<Vec<ColumnInfo>, DbError>>,
    },
    Count {
        table: String,
        filters: Vec<Condition>,
        reply: mpsc::Sender<Result<i64, DbError>>,
    },
    Page {
        req: PageRequest,
        reply: mpsc::Sender<Result<Page, DbError>>,
    },
}

/// A synchronous handle to a database, backed by a dedicated thread running a
/// current-thread tokio runtime. Each query method returns immediately with a
/// [`mpsc::Receiver`] the caller polls with `try_recv()` from the TUI loop.
///
/// Queries are serialised (one connection, one in flight) — fine for a viewer
/// and keeps the bridge trivial. Dropping the handle closes the channel, which
/// ends the worker loop and the pool.
pub struct DbConnection {
    backend: DbBackend,
    tx: Option<mpsc::Sender<Request>>,
    worker: Option<JoinHandle<()>>,
}

impl DbConnection {
    /// Connect to `url`. Blocks until the connection is established (or fails);
    /// callers that must not block the UI should run this on a short-lived
    /// background thread and poll for the resulting handle.
    pub fn connect(url: &str) -> Result<Self, DbError> {
        let backend = DbBackend::from_url(url)?;
        let url = url.to_string();
        let (init_tx, init_rx) = mpsc::channel::<Result<(), DbError>>();
        let (tx, rx) = mpsc::channel::<Request>();

        let worker = thread::Builder::new()
            .name("termide-db".into())
            .spawn(move || run_worker(url, init_tx, rx))
            .map_err(|e| DbError::Runtime(e.to_string()))?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                backend,
                tx: Some(tx),
                worker: Some(worker),
            }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                let _ = worker.join();
                Err(DbError::Closed)
            }
        }
    }

    pub fn backend(&self) -> DbBackend {
        self.backend
    }

    /// List tables. Poll the returned receiver for the result.
    pub fn list_tables(&self) -> mpsc::Receiver<Result<Vec<String>, DbError>> {
        let (reply, rx) = mpsc::channel();
        self.dispatch(Request::ListTables(reply.clone()), || {
            let _ = reply.send(Err(DbError::Closed));
        });
        rx
    }

    /// Describe a table's columns (name + inferred type category).
    pub fn columns(
        &self,
        table: impl Into<String>,
    ) -> mpsc::Receiver<Result<Vec<ColumnInfo>, DbError>> {
        let (reply, rx) = mpsc::channel();
        let table = table.into();
        let fail_reply = reply.clone();
        self.dispatch(
            Request::Columns {
                table,
                reply: reply.clone(),
            },
            move || {
                let _ = fail_reply.send(Err(DbError::Closed));
            },
        );
        rx
    }

    /// Count rows in `table`, honouring `filters` (the filtered total shown in
    /// the status bar).
    pub fn count(
        &self,
        table: impl Into<String>,
        filters: Vec<Condition>,
    ) -> mpsc::Receiver<Result<i64, DbError>> {
        let (reply, rx) = mpsc::channel();
        let table = table.into();
        let fail_reply = reply.clone();
        self.dispatch(
            Request::Count {
                table,
                filters,
                reply: reply.clone(),
            },
            move || {
                let _ = fail_reply.send(Err(DbError::Closed));
            },
        );
        rx
    }

    /// Fetch one page of rows.
    pub fn page(&self, req: PageRequest) -> mpsc::Receiver<Result<Page, DbError>> {
        let (reply, rx) = mpsc::channel();
        let fail_reply = reply.clone();
        self.dispatch(
            Request::Page {
                req,
                reply: reply.clone(),
            },
            move || {
                let _ = fail_reply.send(Err(DbError::Closed));
            },
        );
        rx
    }

    fn dispatch(&self, req: Request, on_closed: impl FnOnce()) {
        match &self.tx {
            Some(tx) if tx.send(req).is_ok() => {}
            _ => on_closed(),
        }
    }
}

impl Drop for DbConnection {
    fn drop(&mut self) {
        // Closing the request channel ends the worker's recv loop, after which
        // it closes the pool and exits.
        self.tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Worker thread body: build a runtime, connect, then serve requests until the
/// channel closes.
fn run_worker(
    url: String,
    init_tx: mpsc::Sender<Result<(), DbError>>,
    rx: mpsc::Receiver<Request>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = init_tx.send(Err(DbError::Runtime(e.to_string())));
            return;
        }
    };

    let pool = match rt.block_on(engine::connect(&url)) {
        Ok(pool) => {
            let _ = init_tx.send(Ok(()));
            pool
        }
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };

    while let Ok(req) = rx.recv() {
        match req {
            Request::ListTables(reply) => {
                let _ = reply.send(rt.block_on(engine::list_tables(&pool)));
            }
            Request::Columns { table, reply } => {
                let _ = reply.send(rt.block_on(engine::columns(&pool, &table)));
            }
            Request::Count {
                table,
                filters,
                reply,
            } => {
                let _ = reply.send(rt.block_on(engine::count_rows(&pool, &table, &filters)));
            }
            Request::Page { req, reply } => {
                let _ = reply.send(rt.block_on(engine::fetch_page(&pool, &req)));
            }
        }
    }

    rt.block_on(engine::close(pool));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sqlite_db() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let setup_url = format!("sqlite://{}?mode=rwc", path.display());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let pool = sqlx::SqlitePool::connect(&setup_url).await.unwrap();
            sqlx::query(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER)",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO users (name, score, active) VALUES \
                 ('alice', 1.5, 1), ('bob', 2.25, 0), ('carol', NULL, 1)",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query("CREATE TABLE empty_t (x INTEGER)")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
        });

        // Read-only URL (no mode=rwc) for the viewer under test.
        (dir, format!("sqlite://{}", path.display()))
    }

    #[test]
    fn backend_from_url() {
        assert_eq!(
            DbBackend::from_url("sqlite:///x.db").unwrap(),
            DbBackend::Sqlite
        );
        assert_eq!(
            DbBackend::from_url("postgres://localhost/db").unwrap(),
            DbBackend::Postgres
        );
        assert_eq!(
            DbBackend::from_url("mysql://h/db").unwrap(),
            DbBackend::MySql
        );
        assert!(DbBackend::from_url("redis://x").is_err());
    }

    #[test]
    fn sqlite_list_tables() {
        let (_dir, url) = make_sqlite_db();
        let conn = DbConnection::connect(&url).unwrap();
        let tables = conn.list_tables().recv().unwrap().unwrap();
        assert_eq!(tables, vec!["empty_t".to_string(), "users".to_string()]);
    }

    #[test]
    fn sqlite_count_and_page() {
        let (_dir, url) = make_sqlite_db();
        let conn = DbConnection::connect(&url).unwrap();

        let total = conn.count("users", vec![]).recv().unwrap().unwrap();
        assert_eq!(total, 3);

        let page = conn
            .page(PageRequest {
                table: "users".into(),
                limit: 2,
                offset: 0,
                ..Default::default()
            })
            .recv()
            .unwrap()
            .unwrap();
        assert_eq!(page.columns, vec!["id", "name", "score", "active"]);
        assert_eq!(page.rows.len(), 2);
        assert!(page.has_more);
        assert_eq!(page.rows[0][0], DbValue::Int(1));
        assert_eq!(page.rows[0][1], DbValue::Text("alice".into()));
        assert_eq!(page.rows[0][2], DbValue::Float(1.5));

        // Second page: the remaining row, NULL preserved, no more pages.
        let page2 = conn
            .page(PageRequest {
                table: "users".into(),
                limit: 2,
                offset: 2,
                ..Default::default()
            })
            .recv()
            .unwrap()
            .unwrap();
        assert_eq!(page2.rows.len(), 1);
        assert!(!page2.has_more);
        assert_eq!(page2.rows[0][1], DbValue::Text("carol".into()));
        assert_eq!(page2.rows[0][2], DbValue::Null);
    }

    #[test]
    fn sqlite_columns_with_categories() {
        let (_dir, url) = make_sqlite_db();
        let conn = DbConnection::connect(&url).unwrap();

        // Works for an empty table too (catalog-derived, not row-derived).
        let cols = conn.columns("empty_t").recv().unwrap().unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, "x");
        assert_eq!(cols[0].category, TypeCategory::Number);

        let cols = conn.columns("users").recv().unwrap().unwrap();
        let by_name = |n: &str| cols.iter().find(|c| c.name == n).unwrap().category;
        assert_eq!(by_name("id"), TypeCategory::Number);
        assert_eq!(by_name("name"), TypeCategory::Text);
        assert_eq!(by_name("score"), TypeCategory::Number); // REAL
        assert_eq!(by_name("active"), TypeCategory::Number); // INTEGER
    }

    #[test]
    fn sqlite_filter_contains_and_compare() {
        let (_dir, url) = make_sqlite_db();
        let conn = DbConnection::connect(&url).unwrap();

        // contains "a" → alice, carol (case-insensitive ASCII).
        let filters = vec![Condition {
            column: "name".into(),
            op: FilterOp::Contains,
            value: Some(DbValue::Text("a".into())),
        }];
        let total = conn
            .count("users", filters.clone())
            .recv()
            .unwrap()
            .unwrap();
        assert_eq!(total, 2);

        let page = conn
            .page(PageRequest {
                table: "users".into(),
                filters,
                order_by: vec![("name".into(), SortDir::Asc)],
                limit: 50,
                offset: 0,
            })
            .recv()
            .unwrap()
            .unwrap();
        let names: Vec<String> = page.rows.iter().map(|r| r[1].display()).collect();
        assert_eq!(names, vec!["alice", "carol"]);

        // score >= 2.0 → only bob.
        let page = conn
            .page(PageRequest {
                table: "users".into(),
                filters: vec![Condition {
                    column: "score".into(),
                    op: FilterOp::Ge,
                    value: Some(DbValue::Float(2.0)),
                }],
                ..Default::default()
            })
            .recv()
            .unwrap()
            .unwrap();
        // limit defaults to 0 → coerced to 1 in engine; bob is the only match.
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0][1], DbValue::Text("bob".into()));
    }

    #[test]
    fn sqlite_filter_is_null_and_sort_desc() {
        let (_dir, url) = make_sqlite_db();
        let conn = DbConnection::connect(&url).unwrap();

        let page = conn
            .page(PageRequest {
                table: "users".into(),
                filters: vec![Condition {
                    column: "score".into(),
                    op: FilterOp::IsNull,
                    value: None,
                }],
                limit: 50,
                ..Default::default()
            })
            .recv()
            .unwrap()
            .unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.rows[0][1], DbValue::Text("carol".into()));

        // Sort by name DESC → carol, bob, alice.
        let page = conn
            .page(PageRequest {
                table: "users".into(),
                order_by: vec![("name".into(), SortDir::Desc)],
                limit: 50,
                ..Default::default()
            })
            .recv()
            .unwrap()
            .unwrap();
        let names: Vec<String> = page.rows.iter().map(|r| r[1].display()).collect();
        assert_eq!(names, vec!["carol", "bob", "alice"]);
    }

    #[test]
    fn connect_rejects_bad_scheme() {
        match DbConnection::connect("redis://localhost") {
            Err(DbError::UnsupportedScheme(_)) => {}
            other => panic!("expected UnsupportedScheme, got {:?}", other.map(|_| ())),
        }
    }
}
