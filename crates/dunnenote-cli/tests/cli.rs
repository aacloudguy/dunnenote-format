//! `dnfmt`'s editing commands, end to end through the binary.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use dunnenote_format::{CanvasKind, Notebook};
use tempfile::TempDir;

fn dnfmt(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dnfmt"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

/// Run a command that must succeed and print one id.
fn id(args: &[&str], stdin: Option<&str>) -> String {
    let out = dnfmt(args, stdin);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert_eq!(id.len(), 36, "{args:?} printed {id:?}");
    id
}

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0xF3, 0xFF,
    0x61, 0x00, 0x00, 0x00, 0x26, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x78, 0xE6, 0x65, 0xF3,
    0x1F, 0x1F, 0xFE, 0xB5, 0x0F, 0x3F, 0x66, 0x18, 0x35, 0x60, 0x58, 0x18, 0x60, 0xB3, 0xE0, 0xD9,
    0x7F, 0x7C, 0x98, 0x10, 0x18, 0x35, 0x60, 0x58, 0x18, 0x00, 0x00, 0x9A, 0x7B, 0x06, 0xEE, 0x0F,
    0xA8, 0xE5, 0xEB, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn build_a_notebook_from_the_command_line() {
    let dir = TempDir::new().unwrap();
    let nb_path = dir.path().join("Trip.dunnenote");
    let nb = nb_path.to_str().unwrap();
    let image = dir.path().join("swatch.png");
    std::fs::write(&image, PNG).unwrap();
    let strokes = dir.path().join("strokes.json");
    std::fs::write(
        &strokes,
        r##"{"v":1,"strokes":[{"id":"a","points":[{"x":0.1,"y":0.1,"p":0.5},{"x":0.9,"y":0.9,"p":0.5}],"color":"#2980b9","width":3,"tool":"pen"}]}"##,
    )
    .unwrap();

    let root = id(&["new", nb, "--name=Summer Trip"], None);
    let plans = id(&["add-section", nb, "root", "Plans"], None);
    let first = id(&["add-section", nb, &root, "Before plans", "--first"], None);
    let page = id(&["add-page", nb, &plans, "Day 1"], None);
    let bare = id(&["add-page", nb, &plans, "Bare", "--no-text"], None);
    let text = id(
        &["add-canvas", nb, &page, "rich-text", "-"],
        Some("# Day 1\n\nTrain at **9:10**.\n"),
    );
    let sketch = id(
        &[
            "add-canvas",
            nb,
            &page,
            "sketch",
            strokes.to_str().unwrap(),
            "--size=300,200",
        ],
        None,
    );
    let picture = id(
        &[
            "add-canvas",
            nb,
            &page,
            "picture",
            image.to_str().unwrap(),
            "--alt=Swatches",
            "--at=400,40",
        ],
        None,
    );

    let verify = dnfmt(&["verify", nb, "--full", "--strict"], None);
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stdout)
    );

    let read = Notebook::open(&nb_path).unwrap();
    assert_eq!(read.notebook_node().unwrap().name, "Summer Trip");
    let sections: Vec<String> = read
        .children(&root)
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(sections, [first, plans]);
    assert!(read.canvases(&bare).unwrap().is_empty());
    let kinds: Vec<CanvasKind> = read
        .canvases(&page)
        .unwrap()
        .iter()
        .map(|c| c.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            CanvasKind::RichText,
            CanvasKind::RichText,
            CanvasKind::Sketch,
            CanvasKind::Picture
        ],
        "the page's own text box, then the three added"
    );
    // Added canvases go below what is already there unless --at is given.
    let below = read.canvas(&text).unwrap();
    assert_eq!((below.x, below.y), (40, 40 + 320 + 20));
    let s = read.canvas(&sketch).unwrap();
    assert_eq!((s.width, s.height), (300, 200));
    let p = read.canvas(&picture).unwrap();
    assert_eq!((p.x, p.y, p.width, p.height), (400, 40, 16, 16));
    assert_eq!(p.settings["alt"], "Swatches");
    let doc = read.rich_text(&text).unwrap().unwrap().doc;
    assert_eq!(doc["content"][0]["type"], "heading");
}

#[test]
fn editing_refuses_what_it_cannot_do_safely() {
    let dir = TempDir::new().unwrap();
    let nb_path = dir.path().join("Held.dunnenote");
    let nb = nb_path.to_str().unwrap();
    id(&["new", nb], None);
    let page = id(&["add-page", nb, "root", "P"], None);

    // Held open by another writer (standing in for DunneNote): refused, nothing changed.
    let holder = Notebook::open_writable(&nb_path).unwrap();
    let out = dnfmt(&["add-section", nb, "root", "S"], None);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("open in another program"));
    drop(holder);

    for args in [
        vec!["new", nb],
        vec!["add-canvas", nb, "root", "table"],
        vec!["add-canvas", nb, &page, "rich-text", "--size=1,2,3"],
        vec!["add-canvas", nb, &page, "rich-text", "--size=0,10"],
        vec!["add-page", nb, &page, "under a page"],
    ] {
        let out = dnfmt(&args, None);
        assert!(!out.status.success(), "{args:?} should fail");
    }
    let read = Notebook::open(&nb_path).unwrap();
    assert_eq!(
        read.walk().unwrap().len(),
        2,
        "still only the root and the page"
    );
    assert_eq!(
        read.canvases(&page).unwrap().len(),
        1,
        "only the page's own text box"
    );
}

#[test]
fn tables_from_the_command_line() {
    let dir = TempDir::new().unwrap();
    let nb_path = dir.path().join("Kit.dunnenote");
    let nb = nb_path.to_str().unwrap();
    let csv = dir.path().join("kit.csv");
    std::fs::write(&csv, "Item,Qty\nTent,2\nStove,1\n").unwrap();
    id(&["new", nb], None);
    let page = id(&["add-page", nb, "root", "Kit", "--no-text"], None);

    let data = id(
        &["add-canvas", nb, &page, "table", csv.to_str().unwrap()],
        None,
    );
    let sheet = id(&["add-canvas", nb, &page, "table"], None);
    let key = dnfmt(&["table", nb, &sheet, "add-column", "Qty"], None);
    assert!(key.status.success());
    assert_eq!(String::from_utf8_lossy(&key.stdout).trim(), "c3");
    let row = id(
        &["table", nb, &sheet, "add-row", "column 1=Rope", "qty=10"],
        None,
    );
    id(&["table", nb, &sheet, "set", &row, "c1", "blue"], None);

    // A Data Table keeps what it was imported with.
    let refused = dnfmt(&["table", nb, &data, "add-row", "Item=Map"], None);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("Editable"));

    let verify = dnfmt(&["verify", nb, "--full", "--strict"], None);
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stdout)
    );
    let read = Notebook::open(&nb_path).unwrap();
    let imported = read.dataset(&data).unwrap().unwrap();
    assert_eq!(imported.source_kind, "csv");
    assert_eq!(imported.row_count, 2);
    let edited = read.dataset(&sheet).unwrap().unwrap();
    assert_eq!(edited.row_count, 4);
    let last = edited.rows.last().unwrap();
    assert_eq!(last.id, row);
    assert_eq!(
        serde_json::Value::Object(last.cells.clone()),
        serde_json::json!({"c0": "Rope", "c3": 10, "c1": "blue"})
    );
}

#[test]
fn forms_and_captions_from_the_command_line() {
    let dir = TempDir::new().unwrap();
    let nb_path = dir.path().join("Survey.dunnenote");
    let nb = nb_path.to_str().unwrap();
    let strokes = dir.path().join("sig.json");
    std::fs::write(
        &strokes,
        r##"[{"id":"s","points":[{"x":0.1,"y":0.5,"p":0.5},{"x":0.9,"y":0.5,"p":0.5}],"color":"#111111","width":2,"tool":"pen"}]"##,
    )
    .unwrap();
    let image = dir.path().join("photo.png");
    std::fs::write(&image, PNG).unwrap();
    id(&["new", nb], None);

    let page = id(&["form", nb, "new", "root", "--name=Visit log"], None);
    let read = Notebook::open(&nb_path).unwrap();
    let text = read.canvases(&page).unwrap()[0].id.clone();
    drop(read);
    let sig = id(&["add-canvas", nb, &page, "sketch"], None);
    for args in [
        vec!["form", nb, "field", &text, "--name=Visitor", "--required"],
        vec![
            "form",
            nb,
            "field",
            &sig,
            "--name=Signature",
            "--label=Sign here",
        ],
    ] {
        let out = dnfmt(&args, None);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let row = id(
        &[
            "form",
            nb,
            "submit",
            &page,
            "visitor=Ada Lovelace",
            &format!("Signature=@{}", strokes.display()),
            "--utc-offset=60",
        ],
        None,
    );
    // Required and blank: refused, nothing appended.
    let refused = dnfmt(&["form", nb, "submit", &page, "Visitor= "], None);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("required"));

    let pic = id(
        &["add-canvas", nb, &page, "picture", image.to_str().unwrap()],
        None,
    );
    let cap = id(&["caption", nb, &pic, "Figure 1", "--placement=top"], None);

    let verify = dnfmt(&["verify", nb, "--full", "--strict"], None);
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stdout)
    );
    let read = Notebook::open(&nb_path).unwrap();
    let table = read
        .canvases(&page)
        .unwrap()
        .into_iter()
        .find(|c| c.kind == CanvasKind::Spreadsheet)
        .unwrap();
    let data = read.dataset(&table.id).unwrap().unwrap();
    assert_eq!(data.rows.len(), 1);
    assert_eq!(data.rows[0].id, row);
    let names: Vec<&str> = data.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Visitor", "Signature", "Submitted"]);
    assert_eq!(data.rows[0].cells["c0"], "Ada Lovelace");
    assert!(data.rows[0].cells["c1"]
        .as_str()
        .unwrap()
        .starts_with("sketch:"));
    assert!(data.rows[0].cells["c2"]
        .as_str()
        .unwrap()
        .ends_with("+01:00"));
    let c = read.canvas(&cap).unwrap();
    assert_eq!(c.settings["caption"]["placement"], "top");
}
