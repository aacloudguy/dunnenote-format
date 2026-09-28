//! Check a notebook for damage and for rule violations.
//!
//! Every check is read-only. [`VerifyLevel::Quick`] runs SQLite's `quick_check` and checks that
//! each blob file exists with the recorded size; [`VerifyLevel::Full`] runs `integrity_check` and
//! re-hashes every blob.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::model::NodeKind;
use crate::notebook::Notebook;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyLevel {
    Quick,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Worth knowing; DunneNote copes with it.
    Info,
    /// Outside the format's rules; DunneNote may repair or ignore it.
    Warning,
    /// Damage: data may be unreadable or lost.
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// Short stable identifier, e.g. `refcount_drift`.
    pub check: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub checks_run: Vec<&'static str>,
    pub findings: Vec<Finding>,
}

impl Report {
    /// No warnings or errors.
    pub fn is_clean(&self) -> bool {
        self.findings.iter().all(|f| f.severity == Severity::Info)
    }
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }
    fn add(&mut self, severity: Severity, check: &'static str, detail: impl Into<String>) {
        self.findings.push(Finding {
            severity,
            check,
            detail: detail.into(),
        });
    }
}

/// Lifecycle reasons DunneNote accepts for archived pages and canvases.
pub const LIFECYCLE_REASONS: [&str; 4] = ["superseded", "wrong", "irrelevant", "other"];

pub fn verify(nb: &Notebook, level: VerifyLevel) -> Result<Report> {
    let conn = nb.connection();
    let mut r = Report::default();

    // 1. SQLite's own consistency check.
    r.checks_run.push("sqlite_integrity");
    let pragma = match level {
        VerifyLevel::Quick => "PRAGMA quick_check",
        VerifyLevel::Full => "PRAGMA integrity_check",
    };
    let lines: Vec<String> = conn
        .prepare(pragma)?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if lines != ["ok"] {
        for line in lines.iter().take(20) {
            r.add(Severity::Error, "sqlite_integrity", line.clone());
        }
    }

    // 2. Foreign keys.
    r.checks_run.push("foreign_keys");
    let fk: Vec<(String, i64, String)> = conn
        .prepare("PRAGMA foreign_key_check")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (table, rowid, parent) in fk.iter().take(20) {
        r.add(
            Severity::Error,
            "foreign_keys",
            format!("{table} row {rowid} points at a missing {parent}"),
        );
    }

    // 3. The page tree.
    r.checks_run.push("tree");
    let roots: i64 = conn.query_row(
        "SELECT count(*) FROM nodes WHERE parent_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    if roots != 1 {
        r.add(
            Severity::Error,
            "tree",
            format!("expected exactly one notebook root, found {roots}"),
        );
    }
    let drift: Vec<(String, i64, i64)> = conn
        .prepare(
            "SELECT n.id, n.child_count, (SELECT count(*) FROM nodes c WHERE c.parent_id = n.id) \
             FROM nodes n WHERE n.child_count <> (SELECT count(*) FROM nodes c WHERE c.parent_id = n.id)",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, stored, actual) in drift {
        r.add(
            Severity::Warning,
            "tree",
            format!("node {id} records {stored} children but has {actual}"),
        );
    }
    let canvases_off_page: i64 = conn.query_row(
        "SELECT count(*) FROM canvas_instances ci JOIN nodes n ON n.id = ci.page_id WHERE n.kind <> 'page'",
        [],
        |row| row.get(0),
    )?;
    if canvases_off_page > 0 {
        r.add(
            Severity::Error,
            "tree",
            format!("{canvases_off_page} canvases belong to something other than a page"),
        );
    }
    if roots == 1 && nb.notebook_node()?.kind != NodeKind::Notebook {
        r.add(Severity::Error, "tree", "the root node is not a notebook");
    }

    // 4. Blob reference counts: each blob's refcount equals the canvases that use it.
    r.checks_run.push("refcounts");
    let drifted: Vec<(String, i64, i64)> = conn
        .prepare(
            "SELECT b.hash, b.refcount, (SELECT count(*) FROM canvas_instances ci WHERE ci.source_hash = b.hash) \
             FROM blobs b WHERE b.refcount <> (SELECT count(*) FROM canvas_instances ci WHERE ci.source_hash = b.hash)",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (hash, stored, actual) in drifted {
        r.add(
            Severity::Warning,
            "refcount_drift",
            format!("blob {hash} records {stored} references but has {actual}"),
        );
    }

    // 5. Blob files.
    r.checks_run.push("blob_files");
    let blobs: Vec<(String, i64, i64)> = conn
        .prepare("SELECT hash, size_bytes, refcount FROM blobs ORDER BY hash")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (hash, size, refcount) in &blobs {
        let path = nb.blob_path(hash)?;
        let severity = if *refcount > 0 {
            Severity::Error
        } else {
            Severity::Info
        };
        match std::fs::metadata(&path) {
            Err(_) => r.add(severity, "blob_files", format!("blob {hash} has no file")),
            Ok(meta) if meta.len() as i64 != *size => r.add(
                severity,
                "blob_files",
                format!("blob {hash} is {} bytes, recorded as {size}", meta.len()),
            ),
            Ok(_) if level == VerifyLevel::Full => {
                let bytes = std::fs::read(&path)?;
                let actual = hex::encode(Sha256::digest(&bytes));
                if &actual != hash {
                    r.add(
                        severity,
                        "blob_hash",
                        format!("blob {hash} content hashes to {actual}"),
                    );
                }
            }
            Ok(_) => {}
        }
    }

    // 6. Content rows each canvas kind needs, and that stored JSON parses.
    r.checks_run.push("canvas_content");
    let canvases: Vec<(String, String, String, String, Option<String>)> = conn
        .prepare("SELECT id, kind, settings, lifecycle, lifecycle_reason FROM canvas_instances")?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let satellites = |table: &str| -> Result<HashMap<String, String>> {
        Ok(conn
            .prepare(&format!("SELECT instance_id, data FROM {table}"))?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    };
    let rich = satellites("rich_text_instances")?;
    let sketches = satellites("sketch_instances")?;
    let datasets: std::collections::HashSet<String> = conn
        .prepare("SELECT instance_id FROM datasets")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, kind, settings, lifecycle, reason) in &canvases {
        if !matches!(
            serde_json::from_str::<Value>(settings),
            Ok(Value::Object(_))
        ) {
            r.add(
                Severity::Warning,
                "canvas_settings",
                format!("canvas {id} settings are not a JSON object"),
            );
        }
        let needs = match kind.as_str() {
            "rich_text" => Some(("rich text", rich.contains_key(id))),
            "sketch" => Some(("sketch", sketches.contains_key(id))),
            "database" | "spreadsheet" => Some(("table", datasets.contains(id))),
            _ => None,
        };
        if let Some((what, present)) = needs {
            if !present {
                r.add(
                    Severity::Warning,
                    "canvas_content",
                    format!("{kind} canvas {id} has no {what} content"),
                );
            }
        }
        if lifecycle != "active" && lifecycle != "archived" {
            r.add(
                Severity::Warning,
                "lifecycle",
                format!("canvas {id} has lifecycle {lifecycle:?}"),
            );
        }
        if let Some(reason) = reason {
            if !LIFECYCLE_REASONS.contains(&reason.as_str()) {
                r.add(
                    Severity::Warning,
                    "lifecycle",
                    format!("canvas {id} has archive reason {reason:?}"),
                );
            }
        }
    }
    for (id, data) in rich.iter() {
        let ok = serde_json::from_str::<Value>(data)
            .ok()
            .is_some_and(|v| v.get("type").and_then(Value::as_str) == Some("doc"));
        if !ok {
            r.add(
                Severity::Error,
                "rich_text",
                format!("rich text of canvas {id} is not a document"),
            );
        }
    }
    for (id, data) in sketches.iter() {
        if serde_json::from_str::<Value>(data).is_err() {
            r.add(
                Severity::Error,
                "sketch",
                format!("sketch data of canvas {id} is not JSON"),
            );
        }
    }

    // 7. Tables: the recorded row count, and cells that belong to a column and are scalars.
    r.checks_run.push("tables");
    let counts: Vec<(String, i64, i64)> = conn
        .prepare(
            "SELECT d.instance_id, d.row_count, (SELECT count(*) FROM dataset_rows r WHERE r.dataset_id = d.id) \
             FROM datasets d WHERE d.row_count <> (SELECT count(*) FROM dataset_rows r WHERE r.dataset_id = d.id)",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (canvas, stored, actual) in counts {
        r.add(
            Severity::Warning,
            "tables",
            format!("table {canvas} records {stored} rows but has {actual}"),
        );
    }
    let stray: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT d.instance_id, r.id, j.key FROM dataset_rows r JOIN datasets d ON d.id = r.dataset_id, \
               json_each(r.cells) j \
             WHERE j.type IN ('object', 'array') OR NOT EXISTS ( \
               SELECT 1 FROM dataset_columns c WHERE c.dataset_id = d.id AND c.col_key = j.key) \
             LIMIT 20",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (canvas, row, key) in stray {
        r.add(
            Severity::Warning,
            "tables",
            format!("table {canvas} row {row} has a cell {key:?} that is not a column value"),
        );
    }

    // 8. Search index: a cache; being empty is normal (DunneNote rebuilds it on open).
    r.checks_run.push("search_index");
    let indexed: i64 = conn.query_row("SELECT count(*) FROM search_index", [], |row| row.get(0))?;
    if indexed == 0 && !canvases.is_empty() {
        r.add(
            Severity::Info,
            "search_index",
            "the search index is empty; DunneNote rebuilds it the next time the notebook opens",
        );
    }

    Ok(r)
}
