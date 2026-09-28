//! Reading and verifying notebooks built directly in the on-disk layout.
//!
//! These fixtures are laid out by hand, exactly as the format specifies. Golden notebooks
//! produced by DunneNote itself are tested separately.

use std::path::{Path, PathBuf};

use dunnenote_format::{
    create_schema, text::plain_text, verify, CanvasKind, Compat, Error, NodeKind, Notebook,
    Severity, VerifyLevel,
};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const NB: &str = "01920000-0000-7000-8000-000000000001";
const SECTION: &str = "01920000-0000-7000-8000-000000000002";
const PAGE: &str = "01920000-0000-7000-8000-000000000003";
const RICH: &str = "01920000-0000-7000-8000-00000000000a";
const SKETCH: &str = "01920000-0000-7000-8000-00000000000b";
const PICTURE: &str = "01920000-0000-7000-8000-00000000000c";
const TABLE: &str = "01920000-0000-7000-8000-00000000000d";
const CAL: &str = "01920000-0000-7000-8000-00000000000e";
const HIDDEN: &str = "01920000-0000-7000-8000-00000000000f";

struct Fixture {
    _dir: TempDir,
    root: PathBuf,
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn put_blob(conn: &Connection, root: &Path, bytes: &[u8]) -> String {
    let hash = sha(bytes);
    let dir = root
        .join("blobs/sha256")
        .join(&hash[0..2])
        .join(&hash[2..4]);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{hash}.bin")), bytes).unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO blobs(hash, size_bytes, created_at, updated_at) VALUES (?1, ?2, unixepoch(), unixepoch())",
        params![hash, bytes.len() as i64],
    )
    .unwrap();
    hash
}

fn canvas(conn: &Connection, id: &str, kind: &str, hash: &str, z: i64, settings: &str) {
    conn.execute(
        "INSERT INTO canvas_instances(id, page_id, kind, source_hash, x, y, width, height, z_index, settings, schema_version, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 10, 20, 300, 200, ?5, ?6, 1, unixepoch(), unixepoch())",
        params![id, PAGE, kind, hash, z, settings],
    )
    .unwrap();
}

/// A notebook with one section, one page and one canvas of every kind (plus a hidden one).
fn build(schema_version: u32) -> Fixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("Fixture.dunnenote");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("format.json"),
        format!(
            r#"{{"format":"dunnenote","format_version":"0.18.0","notebook_id":"{NB}","created_at":1700000000,"created_by":"fixture"}}"#
        ),
    )
    .unwrap();
    let mut conn = Connection::open(root.join("notebook.db")).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    create_schema(&mut conn).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    conn.pragma_update(None, "recursive_triggers", "ON")
        .unwrap();

    let node = |id: &str, kind: &str, parent: Option<&str>, name: &str, pos: &str| {
        conn.execute(
            "INSERT INTO nodes(id, kind, parent_id, name, position, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, unixepoch(), unixepoch())",
            params![id, kind, parent, name, pos],
        )
        .unwrap();
    };
    node(NB, "notebook", None, "Fixture", "80");
    node(SECTION, "group", Some(NB), "Section A", "80");
    node(PAGE, "page", Some(SECTION), "Page One", "80");

    let rich_sentinel = put_blob(&conn, &root, b"calnote:rich_text:blank:v1");
    let sketch_sentinel = put_blob(&conn, &root, b"calnote:sketch:blank:v1");
    let sheet_sentinel = put_blob(&conn, &root, b"calnote:spreadsheet:blank:v1");
    let picture = put_blob(&conn, &root, b"\x89PNG fake picture bytes");
    let ics = put_blob(&conn, &root, b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");

    canvas(
        &conn,
        RICH,
        "rich_text",
        &rich_sentinel,
        0,
        r#"{"frameAppearance":"none","futureKey":{"kept":true}}"#,
    );
    conn.execute(
        "INSERT INTO rich_text_instances(instance_id, data, schema_version, created_at, updated_at) VALUES (?1, ?2, 2, unixepoch(), unixepoch())",
        params![RICH, r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":1},"content":[{"type":"text","text":"Hello"}]},{"type":"paragraph","content":[{"type":"text","text":"World"}]}]}"#],
    ).unwrap();

    canvas(&conn, SKETCH, "sketch", &sketch_sentinel, 1, "{}");
    conn.execute(
        "INSERT INTO sketch_instances(instance_id, data, schema_version, created_at, updated_at) VALUES (?1, ?2, 1, unixepoch(), unixepoch())",
        params![SKETCH, r##"{"v":1,"strokes":[{"id":"s1","points":[{"x":0.1,"y":0.1,"p":0.5},{"x":0.9,"y":0.9,"p":0.5}],"color":"#000000","width":2,"tool":"pen"}]}"##],
    ).unwrap();

    canvas(
        &conn,
        PICTURE,
        "picture",
        &picture,
        2,
        r#"{"alt":"A test picture"}"#,
    );
    conn.execute(
        "INSERT INTO sketch_instances(instance_id, data, schema_version, created_at, updated_at) VALUES (?1, '{\"v\":1,\"strokes\":[]}', 1, unixepoch(), unixepoch())",
        params![PICTURE],
    ).unwrap();

    canvas(&conn, TABLE, "spreadsheet", &sheet_sentinel, 3, "{}");
    conn.execute(
        "INSERT INTO datasets(id, instance_id, shape, source_kind, row_count, schema_version, created_at, updated_at) VALUES ('01920000-0000-7000-8000-0000000000d1', ?1, 'table', 'paste', 2, 1, unixepoch(), unixepoch())",
        params![TABLE],
    ).unwrap();
    for (i, (key, name)) in [("c1", "Name"), ("c2", "Qty")].iter().enumerate() {
        conn.execute(
            "INSERT INTO dataset_columns(id, dataset_id, col_key, name, type_hint, position, created_at, updated_at) VALUES (?1, '01920000-0000-7000-8000-0000000000d1', ?2, ?3, 'text', ?4, unixepoch(), unixepoch())",
            params![format!("01920000-0000-7000-8000-0000000000c{i}"), key, name, i as i64],
        ).unwrap();
    }
    for (seq, cells) in [r#"{"c1":"apples","c2":3}"#, r#"{"c1":"pears"}"#]
        .iter()
        .enumerate()
    {
        conn.execute(
            "INSERT INTO dataset_rows(id, dataset_id, seq, cells, created_at, updated_at) VALUES (?1, '01920000-0000-7000-8000-0000000000d1', ?2, ?3, unixepoch(), unixepoch())",
            params![format!("01920000-0000-7000-8000-0000000000e{seq}"), seq as i64, cells],
        ).unwrap();
    }

    canvas(&conn, CAL, "calendar", &ics, 4, "{}");
    conn.execute(
        "INSERT INTO calendar_events(id, instance_id, summary, location, start_utc, end_utc, created_at, updated_at, source_ordinal) VALUES ('01920000-0000-7000-8000-0000000000f1', ?1, 'Launch', 'Office', 1758000000, 1758003600, unixepoch(), unixepoch(), 0)",
        params![CAL],
    ).unwrap();
    conn.execute(
        "INSERT INTO calendar_event_attendees(id, event_id, ordinal, value, cn, created_at, updated_at) VALUES ('01920000-0000-7000-8000-0000000000f2', '01920000-0000-7000-8000-0000000000f1', 0, 'mailto:a@example.org', 'A', unixepoch(), unixepoch())",
        [],
    ).unwrap();

    canvas(&conn, HIDDEN, "picture", &picture, 5, r#"{"hidden":true}"#);

    conn.execute(
        "INSERT INTO tags(id, name, name_folded, created_at, updated_at) VALUES ('01920000-0000-7000-8000-0000000000a1', 'Texas', 'texas', unixepoch(), unixepoch())",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO tag_aliases(id, alias_name, alias_folded, tag_id, created_at) VALUES ('01920000-0000-7000-8000-0000000000a2', 'TX', 'tx', '01920000-0000-7000-8000-0000000000a1', unixepoch())",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO item_tags(id, tag_id, source_kind, source_id, created_at) VALUES ('01920000-0000-7000-8000-0000000000a3', '01920000-0000-7000-8000-0000000000a1', 'node', ?1, unixepoch())",
        params![PAGE],
    ).unwrap();

    conn.pragma_update(None, "user_version", schema_version)
        .unwrap();
    conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").ok();
    drop(conn);
    Fixture { _dir: dir, root }
}

#[test]
fn reads_the_tree_in_sidebar_order() {
    let f = build(18);
    let nb = Notebook::open(&f.root).unwrap();
    assert_eq!(nb.compat(), Compat::Exact);
    assert_eq!(nb.manifest().notebook_id, NB);
    let walk = nb.walk().unwrap();
    let names: Vec<(usize, &str, NodeKind)> = walk
        .iter()
        .map(|(d, n)| (*d, n.name.as_str(), n.kind))
        .collect();
    assert_eq!(
        names,
        [
            (0, "Fixture", NodeKind::Notebook),
            (1, "Section A", NodeKind::Group),
            (2, "Page One", NodeKind::Page)
        ]
    );
    assert_eq!(nb.pages().unwrap().len(), 1);
}

#[test]
fn reads_every_canvas_kind() {
    let f = build(18);
    let nb = Notebook::open(&f.root).unwrap();
    let canvases = nb.canvases(PAGE).unwrap();
    let kinds: Vec<CanvasKind> = canvases.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            CanvasKind::RichText,
            CanvasKind::Sketch,
            CanvasKind::Picture,
            CanvasKind::Spreadsheet,
            CanvasKind::Calendar,
            CanvasKind::Picture
        ],
        "canvases come bottom layer first"
    );

    // Settings keep unknown keys, in stored order.
    let keys: Vec<&String> = canvases[0].settings.keys().collect();
    assert_eq!(keys, ["frameAppearance", "futureKey"]);

    let doc = nb.rich_text(RICH).unwrap().unwrap().doc;
    assert_eq!(plain_text(&doc), "Hello\nWorld");

    let strokes = nb.sketch(SKETCH).unwrap().unwrap().strokes();
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].points.len(), 2);

    // A picture's markup layer lives in the sketch table under the picture's id.
    assert!(nb.sketch(PICTURE).unwrap().is_some());
    let pic = nb.canvas(PICTURE).unwrap();
    assert_eq!(
        nb.read_blob(&pic.source_hash).unwrap(),
        b"\x89PNG fake picture bytes"
    );

    let ds = nb.dataset(TABLE).unwrap().unwrap();
    let rows: Vec<Vec<String>> = ds.rows.iter().map(|r| ds.row_strings(r)).collect();
    assert_eq!(rows, [vec!["apples", "3"], vec!["pears", ""]]);

    let events = nb.calendar_events(CAL).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "Launch");
    assert_eq!(events[0].attendees[0].cn.as_deref(), Some("A"));

    assert!(nb.canvas(HIDDEN).unwrap().is_hidden());
}

#[test]
fn reads_tags_with_aliases() {
    let f = build(18);
    let nb = Notebook::open(&f.root).unwrap();
    let tags = nb.tags_of("node", PAGE).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "Texas");
    assert_eq!(tags[0].aliases, ["TX"]);
}

#[test]
fn a_well_formed_notebook_verifies_clean() {
    let f = build(18);
    let nb = Notebook::open(&f.root).unwrap();
    for level in [VerifyLevel::Quick, VerifyLevel::Full] {
        let report = verify(&nb, level).unwrap();
        assert!(report.is_clean(), "{:#?}", report.findings);
    }
}

fn open_rw(root: &Path) -> Connection {
    Connection::open(root.join("notebook.db")).unwrap()
}

fn has(report: &dunnenote_format::Report, check: &str, severity: Severity) -> bool {
    report
        .findings
        .iter()
        .any(|f| f.check == check && f.severity == severity)
}

#[test]
fn verify_detects_refcount_drift() {
    let f = build(18);
    open_rw(&f.root)
        .execute("UPDATE blobs SET refcount = refcount + 5", [])
        .unwrap();
    let report = verify(&Notebook::open(&f.root).unwrap(), VerifyLevel::Quick).unwrap();
    assert!(has(&report, "refcount_drift", Severity::Warning));
}

#[test]
fn verify_detects_missing_and_corrupt_blob_files() {
    let f = build(18);
    let nb = Notebook::open(&f.root).unwrap();
    let pic = nb.canvas(PICTURE).unwrap();
    let path = nb.blob_path(&pic.source_hash).unwrap();

    // Same size, different bytes: only a full check sees it.
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    assert!(!verify(&nb, VerifyLevel::Quick).unwrap().has_errors());
    assert!(has(
        &verify(&nb, VerifyLevel::Full).unwrap(),
        "blob_hash",
        Severity::Error
    ));
    assert!(matches!(
        nb.read_blob(&pic.source_hash),
        Err(Error::BlobCorrupt { .. })
    ));

    std::fs::remove_file(&path).unwrap();
    assert!(has(
        &verify(&nb, VerifyLevel::Quick).unwrap(),
        "blob_files",
        Severity::Error
    ));
}

#[test]
fn verify_detects_a_second_root() {
    let f = build(18);
    open_rw(&f.root)
        .execute(
            "INSERT INTO nodes(id, kind, parent_id, name, position, created_at, updated_at) VALUES ('01920000-0000-7000-8000-0000000000ff', 'notebook', NULL, 'Stray', '81', unixepoch(), unixepoch())",
            [],
        )
        .unwrap();
    let nb = Notebook::open(&f.root).unwrap();
    assert!(has(
        &verify(&nb, VerifyLevel::Quick).unwrap(),
        "tree",
        Severity::Error
    ));
    assert!(matches!(nb.notebook_node(), Err(Error::Malformed(_))));
}

#[test]
fn version_gate_bands() {
    assert!(matches!(
        Notebook::open(build(17).root.as_path()),
        Err(Error::SchemaTooOld { found: 17, .. })
    ));
    let f19 = build(19);
    let nb = Notebook::open(&f19.root).unwrap();
    assert_eq!(nb.compat(), Compat::NewerReadOnly { found: 19 });
    assert!(!nb.compat().writable());
    assert!(matches!(
        Notebook::open(build(20).root.as_path()),
        Err(Error::SchemaTooNew { found: 20, .. })
    ));
}

#[test]
fn opening_never_writes_to_the_notebook() {
    let f = build(18);
    let before: Vec<_> = walk_files(&f.root);
    let nb = Notebook::open(&f.root).unwrap();
    verify(&nb, VerifyLevel::Full).unwrap();
    assert!(!nb.locked_by_another_process().unwrap());
    drop(nb);
    let after: Vec<_> = walk_files(&f.root)
        .into_iter()
        .filter(|(p, _)| !p.ends_with("notebook.db-shm") && !p.ends_with("notebook.db-wal"))
        .collect();
    let before: Vec<_> = before
        .into_iter()
        .filter(|(p, _)| !p.ends_with("notebook.db-shm") && !p.ends_with("notebook.db-wal"))
        .collect();
    assert_eq!(before, after, "no file may be created or changed");
    assert!(
        !f.root.join(".dunnenote.lock").exists(),
        "the lock file is never created by a reader"
    );
}

#[test]
fn detects_a_notebook_held_open_by_another_process() {
    let f = build(18);
    let lock = std::fs::File::create(f.root.join(".dunnenote.lock")).unwrap();
    let nb = Notebook::open(&f.root).unwrap();
    assert!(!nb.locked_by_another_process().unwrap());
    lock.lock().unwrap();
    // Held by this process through another handle: flock semantics make it visible.
    assert!(nb.locked_by_another_process().unwrap());
    lock.unlock().unwrap();
    assert!(!nb.locked_by_another_process().unwrap());
}

#[test]
fn rejects_things_that_are_not_notebooks() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        Notebook::open(dir.path()),
        Err(Error::NotANotebook(..))
    ));
    std::fs::write(
        dir.path().join("format.json"),
        r#"{"format":"calnote","format_version":"0.18.0","notebook_id":"x","created_at":0,"created_by":"x"}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("notebook.db"), b"").unwrap();
    assert!(matches!(
        Notebook::open(dir.path()),
        Err(Error::ManifestInvalid(_))
    ));
}

fn walk_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}
