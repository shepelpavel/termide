//! Live PostgreSQL / MySQL integration tests.
//!
//! These exercise the engine-specific SQL that the hermetic SQLite tests can't
//! reach: `information_schema` catalog queries, `ILIKE`/`LIKE`, `$1` vs `?`
//! placeholders, and per-engine type-category mapping.
//!
//! They are **local-only and opt-in**: each test reads a connection URL from an
//! environment variable and skips (no-op) when it isn't set, so CI — which
//! never sets them — simply passes. To run them locally against a throwaway
//! database:
//!
//! ```sh
//! TERMIDE_TEST_POSTGRES_URL=postgres://user:pass@localhost/testdb \
//! TERMIDE_TEST_MYSQL_URL=mysql://user:pass@localhost/testdb \
//!     cargo test -p termide-db --test live_engines -- --nocapture
//! ```
//!
//! Each test creates and drops its own `termide_it_users` table.

use termide_db::{Condition, DbConnection, DbValue, FilterOp, PageRequest, SortDir, TypeCategory};

fn env_url(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

/// Shared assertions once `termide_it_users` exists with the canonical rows:
/// (alice, 1.5, true), (bob, 2.25, false), (carol, NULL, true).
fn assert_browse(conn: &DbConnection, active_is_bool: bool) {
    let tables = conn.list_tables().recv().unwrap().unwrap();
    assert!(
        tables.iter().any(|t| t == "termide_it_users"),
        "table should be listed, got {tables:?}"
    );

    let cols = conn.columns("termide_it_users").recv().unwrap().unwrap();
    let cat = |n: &str| cols.iter().find(|c| c.name == n).unwrap().category;
    assert_eq!(cat("id"), TypeCategory::Number);
    assert_eq!(cat("name"), TypeCategory::Text);
    assert_eq!(cat("score"), TypeCategory::Number);
    assert_eq!(
        cat("active"),
        if active_is_bool {
            TypeCategory::Bool
        } else {
            TypeCategory::Number
        }
    );

    let total = conn
        .count("termide_it_users", vec![])
        .recv()
        .unwrap()
        .unwrap();
    assert_eq!(total, 3);

    // Case-insensitive "contains a" → alice, carol.
    let filters = vec![Condition {
        column: "name".into(),
        op: FilterOp::Contains,
        value: Some(DbValue::Text("A".into())),
    }];
    let filtered = conn
        .count("termide_it_users", filters.clone())
        .recv()
        .unwrap()
        .unwrap();
    assert_eq!(filtered, 2, "case-insensitive contains should match 2 rows");

    let page = conn
        .page(PageRequest {
            table: "termide_it_users".into(),
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

    // NULL preserved, numeric comparison works.
    let ge = conn
        .page(PageRequest {
            table: "termide_it_users".into(),
            filters: vec![Condition {
                column: "score".into(),
                op: FilterOp::Ge,
                value: Some(DbValue::Float(2.0)),
            }],
            order_by: vec![],
            limit: 50,
            offset: 0,
        })
        .recv()
        .unwrap()
        .unwrap();
    assert_eq!(ge.rows.len(), 1);
    assert_eq!(ge.rows[0][1], DbValue::Text("bob".into()));
}

#[test]
fn postgres_browse_filter_sort() {
    let Some(url) = env_url("TERMIDE_TEST_POSTGRES_URL") else {
        eprintln!("skip: set TERMIDE_TEST_POSTGRES_URL to run the PostgreSQL test");
        return;
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::query("DROP TABLE IF EXISTS termide_it_users")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE termide_it_users (\
             id serial PRIMARY KEY, name text, score double precision, active boolean)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO termide_it_users (name, score, active) VALUES \
             ('alice', 1.5, true), ('bob', 2.25, false), ('carol', NULL, true)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    });

    let conn = DbConnection::connect(&url).unwrap();
    assert_browse(&conn, true);
    drop(conn);

    rt.block_on(async {
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        sqlx::query("DROP TABLE termide_it_users")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    });
}

#[test]
fn mysql_browse_filter_sort() {
    let Some(url) = env_url("TERMIDE_TEST_MYSQL_URL") else {
        eprintln!("skip: set TERMIDE_TEST_MYSQL_URL to run the MySQL test");
        return;
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
        sqlx::query("DROP TABLE IF EXISTS termide_it_users")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE termide_it_users (\
             id INT AUTO_INCREMENT PRIMARY KEY, name TEXT, score DOUBLE, active TINYINT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO termide_it_users (name, score, active) VALUES \
             ('alice', 1.5, 1), ('bob', 2.25, 0), ('carol', NULL, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    });

    let conn = DbConnection::connect(&url).unwrap();
    // MySQL has no real boolean — TINYINT maps to the Number category.
    assert_browse(&conn, false);
    drop(conn);

    rt.block_on(async {
        let pool = sqlx::MySqlPool::connect(&url).await.unwrap();
        sqlx::query("DROP TABLE termide_it_users")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    });
}
