//! Tables: Data Tables (imported, read-only) and Editable tables.
//!
//! A table canvas has one `datasets` row with its columns and rows. Its kind decides whether it
//! may change: only an Editable table (`spreadsheet`) is edited; a Data Table (`database`) keeps
//! the rows it was imported with. Column keys are `c0`, `c1`, …; a new column takes one more than
//! the highest numeric key. A new row takes the next `seq` after the highest; deleting a row
//! leaves a gap. `row_count` is recounted in the same transaction as every row change.

use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

use super::{new_id, Frame, Writer, SPREADSHEET_SENTINEL};
use crate::error::{Error, Result};
use crate::ingest::{self, NewColumn, TypeHint, MAX_CELL_BYTES, MAX_COLUMNS};
use crate::model::{CanvasKind, Settings};

/// The `schema_version` DunneNote writes on datasets.
pub const DATASET_SCHEMA_VERSION: i64 = 1;

/// The frame DunneNote gives a new table.
pub const TABLE_SIZE: (i64, i64) = (480, 360);

const BLANK_COLUMN_NAMES: [&str; 3] = ["Column 1", "Column 2", "Column 3"];
const BLANK_ROW_COUNT: usize = 3;

fn valid_col_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn check_cell(key: &str, value: &Value) -> Result<()> {
    let len = match value {
        Value::Object(_) | Value::Array(_) => {
            return Err(Error::Invalid(format!(
                "cell {key} must be text, a number, true/false or null"
            )))
        }
        Value::String(s) => s.len(),
        other => other.to_string().len(),
    };
    if len > MAX_CELL_BYTES {
        return Err(Error::Invalid(format!(
            "cell {key} is {len} bytes; at most {MAX_CELL_BYTES}"
        )));
    }
    Ok(())
}

impl Writer<'_> {
    /// Create the `datasets` row and its columns and rows for a new table canvas.
    pub(crate) fn insert_dataset(
        &mut self,
        canvas: &str,
        source_kind: &str,
        columns: &[NewColumn],
        rows: &[Map<String, Value>],
    ) -> Result<String> {
        let dataset = new_id();
        self.tx.execute(
            "INSERT INTO datasets(id, instance_id, shape, source_kind, row_count, schema_version, \
             created_at, updated_at) VALUES (?1, ?2, 'table', ?3, ?4, ?5, unixepoch(), unixepoch())",
            params![
                dataset,
                canvas,
                source_kind,
                rows.len() as i64,
                DATASET_SCHEMA_VERSION
            ],
        )?;
        for c in columns {
            if !valid_col_key(&c.col_key) || c.position < 0 {
                return Err(Error::Invalid(format!("bad column key {:?}", c.col_key)));
            }
            self.tx.execute(
                "INSERT INTO dataset_columns(id, dataset_id, col_key, name, type_hint, position, \
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), unixepoch())",
                params![
                    new_id(),
                    dataset,
                    c.col_key,
                    c.name,
                    c.type_hint.as_str(),
                    c.position
                ],
            )?;
        }
        for (seq, cells) in rows.iter().enumerate() {
            self.tx.execute(
                "INSERT INTO dataset_rows(id, dataset_id, seq, cells, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, json(?4), unixepoch(), unixepoch())",
                params![
                    new_id(),
                    dataset,
                    seq as i64,
                    Value::Object(cells.clone()).to_string()
                ],
            )?;
        }
        Ok(dataset)
    }

    /// Add an empty Editable table, as DunneNote's "New table" does: three columns named
    /// "Column 1" to "Column 3" and three empty rows.
    pub fn add_table(&mut self, page: &str, frame: Frame, settings: &Settings) -> Result<String> {
        let hash = self.put_blob(SPREADSHEET_SENTINEL)?;
        let id = self.insert_canvas(page, CanvasKind::Spreadsheet, &hash, frame, settings)?;
        let columns: Vec<NewColumn> = BLANK_COLUMN_NAMES
            .iter()
            .enumerate()
            .map(|(i, name)| NewColumn {
                col_key: format!("c{i}"),
                name: (*name).into(),
                type_hint: TypeHint::Unknown,
                position: i as i64,
            })
            .collect();
        self.insert_dataset(&id, "paste", &columns, &vec![Map::new(); BLANK_ROW_COUNT])?;
        Ok(id)
    }

    /// Add a Data Table imported from a CSV file (see [`ingest::parse_csv`]). The file itself is
    /// kept as the canvas's source blob.
    pub fn add_table_from_csv(
        &mut self,
        page: &str,
        frame: Frame,
        csv: &[u8],
        settings: &Settings,
    ) -> Result<String> {
        let parsed = ingest::parse_csv(csv)?;
        self.add_imported(page, frame, csv, "csv", &parsed, settings)
    }

    /// Add a Data Table imported from a JSON or JSON Lines file (see [`ingest::parse_json`]).
    pub fn add_table_from_json(
        &mut self,
        page: &str,
        frame: Frame,
        json: &[u8],
        settings: &Settings,
    ) -> Result<String> {
        let parsed = ingest::parse_json(json)?;
        self.add_imported(page, frame, json, "json", &parsed, settings)
    }

    fn add_imported(
        &mut self,
        page: &str,
        frame: Frame,
        bytes: &[u8],
        source_kind: &str,
        parsed: &ingest::ParsedTable,
        settings: &Settings,
    ) -> Result<String> {
        let hash = self.put_blob(bytes)?;
        let id = self.insert_canvas(page, CanvasKind::Database, &hash, frame, settings)?;
        self.insert_dataset(&id, source_kind, &parsed.columns, &parsed.rows)?;
        Ok(id)
    }

    /// The dataset of an Editable table; a Data Table (or any other canvas) is refused.
    fn editable_dataset(&self, table: &str) -> Result<String> {
        let (kind, _) = self.canvas_kind(table)?;
        if kind != CanvasKind::Spreadsheet {
            return Err(Error::Invalid(format!(
                "canvas {table} is a {}; only an Editable table can be changed",
                kind.display_name()
            )));
        }
        self.tx
            .query_row(
                "SELECT id FROM datasets WHERE instance_id = ?1",
                [table],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::Malformed(format!("table {table} has no dataset")))
    }

    fn touch_dataset(&self, dataset: &str, recount: bool) -> Result<()> {
        let sql = if recount {
            "UPDATE datasets SET row_count = (SELECT count(*) FROM dataset_rows WHERE dataset_id = ?1), \
             updated_at = unixepoch() WHERE id = ?1"
        } else {
            "UPDATE datasets SET updated_at = unixepoch() WHERE id = ?1"
        };
        self.tx.execute(sql, [dataset])?;
        Ok(())
    }

    fn require_column(&self, dataset: &str, col_key: &str) -> Result<()> {
        let found = self
            .tx
            .query_row(
                "SELECT 1 FROM dataset_columns WHERE dataset_id = ?1 AND col_key = ?2",
                [dataset, col_key],
                |_| Ok(()),
            )
            .optional()?;
        found.ok_or_else(|| Error::NotFound {
            kind: "column",
            id: col_key.into(),
        })
    }

    /// The key DunneNote gives the next column: one more than the highest `c<number>` key.
    fn next_col_key(&self, dataset: &str) -> Result<String> {
        let keys: Vec<String> = self
            .tx
            .prepare("SELECT col_key FROM dataset_columns WHERE dataset_id = ?1")?
            .query_map([dataset], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let max = keys
            .iter()
            .filter_map(|k| k.strip_prefix('c')?.parse::<i64>().ok())
            .max()
            .unwrap_or(-1);
        Ok(format!("c{}", max + 1))
    }

    /// Add a column to an Editable table and return its key. Without `position` it goes after
    /// the last column. Existing rows get no value for it.
    pub fn add_column(
        &mut self,
        table: &str,
        name: &str,
        type_hint: TypeHint,
        position: Option<i64>,
    ) -> Result<String> {
        let dataset = self.editable_dataset(table)?;
        let (count, last): (i64, Option<i64>) = self.tx.query_row(
            "SELECT count(*), max(position) FROM dataset_columns WHERE dataset_id = ?1",
            [&dataset],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count as usize >= MAX_COLUMNS {
            return Err(Error::Invalid(format!(
                "a table has at most {MAX_COLUMNS} columns"
            )));
        }
        let position = position.unwrap_or_else(|| last.map_or(0, |p| p + 1));
        if position < 0 {
            return Err(Error::Invalid(
                "a column position cannot be negative".into(),
            ));
        }
        let key = self.next_col_key(&dataset)?;
        self.tx.execute(
            "INSERT INTO dataset_columns(id, dataset_id, col_key, name, type_hint, position, \
             created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), unixepoch())",
            params![new_id(), dataset, key, name, type_hint.as_str(), position],
        )?;
        self.touch_dataset(&dataset, false)?;
        Ok(key)
    }

    /// Rename a column of an Editable table.
    pub fn rename_column(&mut self, table: &str, col_key: &str, name: &str) -> Result<()> {
        let dataset = self.editable_dataset(table)?;
        self.require_column(&dataset, col_key)?;
        self.tx.execute(
            "UPDATE dataset_columns SET name = ?3, updated_at = unixepoch() \
             WHERE dataset_id = ?1 AND col_key = ?2",
            params![dataset, col_key, name],
        )?;
        self.touch_dataset(&dataset, false)
    }

    /// Set a column's `position` (columns show in `position`, then key, order).
    pub fn move_column(&mut self, table: &str, col_key: &str, position: i64) -> Result<()> {
        if position < 0 {
            return Err(Error::Invalid(
                "a column position cannot be negative".into(),
            ));
        }
        let dataset = self.editable_dataset(table)?;
        self.require_column(&dataset, col_key)?;
        self.tx.execute(
            "UPDATE dataset_columns SET position = ?3, updated_at = unixepoch() \
             WHERE dataset_id = ?1 AND col_key = ?2",
            params![dataset, col_key, position],
        )?;
        self.touch_dataset(&dataset, false)
    }

    /// Delete a column of an Editable table, and its value from every row.
    pub fn delete_column(&mut self, table: &str, col_key: &str) -> Result<()> {
        let dataset = self.editable_dataset(table)?;
        self.require_column(&dataset, col_key)?;
        self.tx.execute(
            "DELETE FROM dataset_columns WHERE dataset_id = ?1 AND col_key = ?2",
            params![dataset, col_key],
        )?;
        self.tx.execute(
            "UPDATE dataset_rows SET cells = json_remove(cells, '$.\"' || ?2 || '\"'), \
             updated_at = unixepoch() WHERE dataset_id = ?1",
            params![dataset, col_key],
        )?;
        self.touch_dataset(&dataset, false)
    }

    /// Append a row to an Editable table and return its id. `cells` maps column keys to text,
    /// numbers, booleans or null; every key must be one of the table's columns.
    pub fn insert_row(&mut self, table: &str, cells: &Map<String, Value>) -> Result<String> {
        let dataset = self.editable_dataset(table)?;
        for (key, value) in cells {
            check_cell(key, value)?;
            self.require_column(&dataset, key)?;
        }
        let id = new_id();
        self.tx.execute(
            "INSERT INTO dataset_rows(id, dataset_id, seq, cells, created_at, updated_at) \
             VALUES (?1, ?2, (SELECT coalesce(max(seq), -1) + 1 FROM dataset_rows WHERE dataset_id = ?2), \
               json(?3), unixepoch(), unixepoch())",
            params![id, dataset, Value::Object(cells.clone()).to_string()],
        )?;
        self.touch_dataset(&dataset, true)?;
        Ok(id)
    }

    /// Set one cell of an Editable table.
    pub fn set_cell(&mut self, table: &str, row: &str, col_key: &str, value: &Value) -> Result<()> {
        let dataset = self.editable_dataset(table)?;
        check_cell(col_key, value)?;
        self.require_column(&dataset, col_key)?;
        let n = self.tx.execute(
            "UPDATE dataset_rows SET cells = json_set(cells, '$.\"' || ?3 || '\"', json(?4)), \
             updated_at = unixepoch() WHERE id = ?1 AND dataset_id = ?2",
            params![row, dataset, col_key, value.to_string()],
        )?;
        if n == 0 {
            return Err(Error::NotFound {
                kind: "row",
                id: row.into(),
            });
        }
        self.touch_dataset(&dataset, false)
    }

    /// Delete a row of an Editable table, with its tags and metadata.
    pub fn delete_row(&mut self, table: &str, row: &str) -> Result<()> {
        let dataset = self.editable_dataset(table)?;
        let n = self.tx.execute(
            "DELETE FROM dataset_rows WHERE id = ?1 AND dataset_id = ?2",
            params![row, dataset],
        )?;
        if n == 0 {
            return Err(Error::NotFound {
                kind: "row",
                id: row.into(),
            });
        }
        for table in ["item_tags", "item_meta"] {
            self.tx.execute(
                &format!(
                    "DELETE FROM {table} WHERE source_kind = 'dataset_row' AND source_id = ?1"
                ),
                [row],
            )?;
        }
        self.touch_dataset(&dataset, true)
    }
}
