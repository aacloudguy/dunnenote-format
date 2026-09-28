//! The embedded schema creates a database whose schema is exactly the published file.

use dunnenote_format::schema::SCHEMA_SHA256;
use dunnenote_format::{create_schema, SCHEMA_SQL, SCHEMA_VERSION};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

const TYPE_ORDER: [&str; 4] = ["table", "view", "index", "trigger"];
const FTS5_SHADOW_SUFFIXES: [&str; 5] = ["_data", "_idx", "_content", "_docsize", "_config"];

/// Same canonical dump DunneNote uses to generate `schema/v18.sql`.
fn dump(conn: &Connection) -> String {
    let mut rows: Vec<(String, String, String)> = conn
        .prepare("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let vts: Vec<String> = rows
        .iter()
        .filter(|(_, _, sql)| {
            sql.trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE VIRTUAL TABLE")
        })
        .map(|(_, n, _)| n.clone())
        .collect();
    rows.retain(|(k, n, _)| {
        !(k == "table"
            && vts.iter().any(|vt| {
                FTS5_SHADOW_SUFFIXES
                    .iter()
                    .any(|s| n == &format!("{vt}{s}"))
            }))
    });
    let rank = |k: &str| {
        TYPE_ORDER
            .iter()
            .position(|t| *t == k)
            .unwrap_or(TYPE_ORDER.len())
    };
    rows.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.1.cmp(&b.1)));
    rows.into_iter()
        .map(|(_, _, sql)| format!("{};\n\n", sql.trim_end()))
        .collect()
}

#[test]
fn created_schema_matches_the_published_file() {
    let mut conn = Connection::open_in_memory().unwrap();
    create_schema(&mut conn).unwrap();

    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    let body = SCHEMA_SQL
        .split_once("\n\n")
        .map(|(_, rest)| rest)
        .expect("schema file has a comment header");
    assert_eq!(
        dump(&conn),
        body,
        "created schema must equal schema/v18.sql"
    );
}

#[test]
fn schema_has_the_core_tables() {
    let mut conn = Connection::open_in_memory().unwrap();
    create_schema(&mut conn).unwrap();
    for table in [
        "nodes",
        "canvas_instances",
        "blobs",
        "rich_text_instances",
        "sketch_instances",
        "datasets",
        "dataset_columns",
        "dataset_rows",
        "calendar_events",
        "groups",
        "tags",
        "item_tags",
        "item_meta",
        "search_index",
    ] {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "missing table {table}");
    }
}

#[test]
fn published_schema_is_the_pinned_file() {
    let actual = hex::encode(Sha256::digest(SCHEMA_SQL.as_bytes()));
    assert_eq!(
        actual, SCHEMA_SHA256,
        "schema/v18.sql changed; copy it from DunneNote's docs/format-schema/ and update SCHEMA_SHA256"
    );
}
