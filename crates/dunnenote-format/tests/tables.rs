//! Tables: importing, the blank Editable table, and editing.
//!
//! The reference is DunneNote's own golden notebooks: importing the same CSV must give the same
//! columns and cells, and a blank table must have the same shape.

use std::path::{Path, PathBuf};

use dunnenote_format::ingest::TypeHint;
use dunnenote_format::{CanvasKind, Dataset, Error, Frame, Notebook, Settings};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

#[path = "support/common.rs"]
mod common;
use common::{assert_clean, count, new_notebook, page_in, raw};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// The first table canvas of `kind` in a golden notebook, with its dataset and source bytes.
fn golden_table(notebook: &str, kind: CanvasKind) -> (Dataset, Vec<u8>) {
    let nb = Notebook::open(fixtures().join(format!("{notebook}.dunnenote"))).unwrap();
    for page in nb.pages().unwrap() {
        for c in nb.canvases(&page.id).unwrap() {
            if c.kind == kind {
                let bytes = nb.read_blob(&c.source_hash).unwrap();
                return (nb.dataset(&c.id).unwrap().unwrap(), bytes);
            }
        }
    }
    panic!("no {kind:?} in {notebook}");
}

type Columns = Vec<(String, String, String, i64)>;
type Rows = Vec<(i64, Map<String, Value>)>;

fn shape(d: &Dataset) -> (Columns, Rows) {
    (
        d.columns
            .iter()
            .map(|c| {
                (
                    c.col_key.clone(),
                    c.name.clone(),
                    c.type_hint.clone(),
                    c.position,
                )
            })
            .collect(),
        d.rows.iter().map(|r| (r.seq, r.cells.clone())).collect(),
    )
}

fn frame() -> Frame {
    Frame::new(40, 400, 480, 360)
}

#[test]
fn a_csv_import_matches_dunnenotes() {
    let (golden, csv) = golden_table("every-kind", CanvasKind::Database);
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Import");
    let page = page_in(&mut nb, &root);
    let table = nb
        .write(|w| w.add_table_from_csv(&page, frame(), &csv, &Settings::new()))
        .unwrap();
    let path = nb.root().to_path_buf();
    drop(nb);

    let nb = Notebook::open(&path).unwrap();
    let canvas = nb.canvas(&table).unwrap();
    assert_eq!(canvas.kind, CanvasKind::Database);
    assert_eq!(
        canvas.source_hash,
        "5c587aa5".to_string() + &canvas.source_hash[8..]
    );
    let ours = nb.dataset(&table).unwrap().unwrap();
    assert_eq!(
        (ours.shape.as_str(), ours.source_kind.as_str()),
        ("table", "csv")
    );
    assert_eq!(ours.row_count, golden.row_count);
    assert_eq!(shape(&ours), shape(&golden));
    assert_clean(&path);
}

#[test]
fn a_blank_table_matches_dunnenotes() {
    let (golden, sentinel) = golden_table("every-kind", CanvasKind::Spreadsheet);
    assert_eq!(sentinel, b"calnote:spreadsheet:blank:v1");
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Blank");
    let page = page_in(&mut nb, &root);
    let table = nb
        .write(|w| w.add_table(&page, frame(), &Settings::new()))
        .unwrap();
    let ours = nb.dataset(&table).unwrap().unwrap();
    assert_eq!(
        (ours.shape.as_str(), ours.source_kind.as_str()),
        ("table", "paste")
    );
    // The golden table was typed into after it was created; its columns and row slots are the
    // blank table's.
    let (golden_cols, golden_rows) = shape(&golden);
    let (cols, rows) = shape(&ours);
    assert_eq!(cols, golden_cols);
    let seqs = |r: &[(i64, Map<String, Value>)]| r.iter().map(|x| x.0).collect::<Vec<_>>();
    assert_eq!(seqs(&rows), seqs(&golden_rows));
    assert!(rows.iter().all(|(_, cells)| cells.is_empty()));
    assert_eq!(ours.row_count, 3);
    assert_clean(nb.root());
}

#[test]
fn json_imports_keep_types_and_the_file() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Json");
    let page = page_in(&mut nb, &root);
    let file = br#"[{"Item":"Tent","Qty":2,"Packed":true},{"Item":"Stove","Qty":1}]"#;
    let table = nb
        .write(|w| w.add_table_from_json(&page, frame(), file, &Settings::new()))
        .unwrap();
    let d = nb.dataset(&table).unwrap().unwrap();
    assert_eq!(d.source_kind, "json");
    let hints: Vec<&str> = d.columns.iter().map(|c| c.type_hint.as_str()).collect();
    assert_eq!(hints, ["text", "number", "boolean"]);
    assert_eq!(
        Value::Object(d.rows[1].cells.clone()),
        json!({"c0": "Stove", "c1": 1})
    );
    let canvas = nb.canvas(&table).unwrap();
    assert_eq!(nb.read_blob(&canvas.source_hash).unwrap(), file);

    let bad = nb.write(|w| w.add_table_from_json(&page, frame(), b"{nope", &Settings::new()));
    assert!(matches!(bad, Err(Error::Invalid(_))), "{bad:?}");
    assert_clean(nb.root());
}

#[test]
fn editing_an_editable_table() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Edit");
    let page = page_in(&mut nb, &root);
    let path = nb.root().to_path_buf();
    let table = nb
        .write(|w| w.add_table(&page, frame(), &Settings::new()))
        .unwrap();
    let rows_before: Vec<String> = nb
        .dataset(&table)
        .unwrap()
        .unwrap()
        .rows
        .iter()
        .map(|r| r.id.clone())
        .collect();

    let (key, new_row) = nb
        .write(|w| {
            w.rename_column(&table, "c0", "Name")?;
            let key = w.add_column(&table, "Hours", TypeHint::Number, None)?;
            w.set_cell(&table, &rows_before[0], "c0", &json!("Ada"))?;
            w.set_cell(&table, &rows_before[0], &key, &json!(7.5))?;
            let mut cells = Map::new();
            cells.insert("c0".into(), json!("Grace"));
            cells.insert(key.clone(), json!(3));
            let row = w.insert_row(&table, &cells)?;
            w.delete_row(&table, &rows_before[1])?;
            w.set_cell(
                &table,
                &rows_before[0],
                "c2",
                &json!("gone with its column"),
            )?;
            w.delete_column(&table, "c2")?;
            w.move_column(&table, &key, 0)?;
            Ok((key, row))
        })
        .unwrap();
    assert_eq!(key, "c3");

    let d = nb.dataset(&table).unwrap().unwrap();
    assert_eq!(d.row_count, 3);
    let seqs: Vec<i64> = d.rows.iter().map(|r| r.seq).collect();
    assert_eq!(
        seqs,
        [0, 2, 3],
        "a deleted row leaves a gap; new rows go after the last"
    );
    assert_eq!(d.rows[2].id, new_row);
    let cols: Vec<(&str, &str, i64)> = d
        .columns
        .iter()
        .map(|c| (c.col_key.as_str(), c.name.as_str(), c.position))
        .collect();
    assert_eq!(
        cols,
        [("c0", "Name", 0), ("c3", "Hours", 0), ("c1", "Column 2", 1)]
    );
    assert_eq!(
        Value::Object(d.rows[0].cells.clone()),
        json!({"c0": "Ada", "c3": 7.5})
    );
    // The key after the highest, even once c2 has gone.
    let next = nb
        .write(|w| w.add_column(&table, "Notes", TypeHint::Text, None))
        .unwrap();
    assert_eq!(next, "c4");
    assert_clean(&path);

    // Stored exactly: a JSON object with the app's own number text.
    let stored: String = raw(&path)
        .query_row(
            "SELECT cells FROM dataset_rows WHERE id = ?1",
            [&rows_before[0]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, r#"{"c0":"Ada","c3":7.5}"#);
}

#[test]
fn deleting_a_row_drops_its_tags_and_metadata() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "RowTags");
    let page = page_in(&mut nb, &root);
    let table = nb
        .write(|w| w.add_table(&page, frame(), &Settings::new()))
        .unwrap();
    let row = nb.dataset(&table).unwrap().unwrap().rows[0].id.clone();
    let path = nb.root().to_path_buf();
    nb.write(|w| {
        let tx = w.transaction();
        tx.execute(
            "INSERT INTO tags(id, name, name_folded, created_at, updated_at) \
             VALUES ('01a0e92f-0000-7000-8000-000000000001', 'x', 'x', 1, 1)",
            [],
        )?;
        tx.execute(
            "INSERT INTO item_tags(id, tag_id, source_kind, source_id, created_at) \
             VALUES ('01a0e92f-0000-7000-8000-000000000002', '01a0e92f-0000-7000-8000-000000000001', 'dataset_row', ?1, 1)",
            [&row],
        )?;
        Ok(())
    })
    .unwrap();
    nb.write(|w| w.delete_row(&table, &row)).unwrap();
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM item_tags WHERE source_kind = 'dataset_row'"
        ),
        0
    );
}

#[test]
fn data_tables_and_bad_cells_are_refused() {
    let (_, csv) = golden_table("every-kind", CanvasKind::Database);
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Refuse");
    let page = page_in(&mut nb, &root);
    let (data, editable) = nb
        .write(|w| {
            Ok((
                w.add_table_from_csv(&page, frame(), &csv, &Settings::new())?,
                w.add_table(&page, frame(), &Settings::new())?,
            ))
        })
        .unwrap();
    let before = count(nb.root(), "SELECT count(*) FROM dataset_rows");

    let read_only = nb.write(|w| w.insert_row(&data, &Map::new()));
    assert!(
        matches!(read_only, Err(Error::Invalid(ref m)) if m.contains("Editable")),
        "{read_only:?}"
    );
    let row = nb.dataset(&editable).unwrap().unwrap().rows[0].id.clone();
    for value in [
        json!({"a": 1}),
        json!([1]),
        json!("x".repeat(64 * 1024 + 1)),
    ] {
        let r = nb.write(|w| w.set_cell(&editable, &row, "c0", &value));
        assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
    }
    let unknown = nb.write(|w| w.set_cell(&editable, &row, "c9", &json!(1)));
    assert!(
        matches!(unknown, Err(Error::NotFound { .. })),
        "{unknown:?}"
    );
    let mut cells = Map::new();
    cells.insert("nope".into(), json!(1));
    assert!(nb.write(|w| w.insert_row(&editable, &cells)).is_err());
    assert_eq!(
        count(nb.root(), "SELECT count(*) FROM dataset_rows"),
        before
    );
    assert_clean(nb.root());
}
