//! Hostile notebooks (`SPEC.md` §15): values read from a notebook never become paths outside it,
//! and exports never carry links DunneNote would refuse to follow.

use dunnenote_format::write::ArchiveReason;
use dunnenote_format::{export, text, Frame, Settings, Stroke, StrokePoint};
use serde_json::json;
use tempfile::TempDir;

#[path = "support/common.rs"]
mod common;
use common::*;

#[test]
fn a_crafted_node_id_cannot_place_a_snapshot_outside_the_notebook() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Hostile");
    // 36 characters, as the schema requires, but a path: `.archive/../../<id>` is `dir`.
    let id = format!("../../{}", "e".repeat(30));
    assert_eq!(id.len(), 36);
    raw(nb.root())
        .execute(
            "INSERT INTO nodes(id, kind, parent_id, name, position, child_count, is_template, \
             created_at, updated_at) VALUES (?1, 'page', ?2, 'Evil', 'c080', 0, 0, unixepoch(), \
             unixepoch())",
            [&id, &root],
        )
        .unwrap();

    let err = nb
        .write(|w| w.archive_node(&id, ArchiveReason::Superseded, None))
        .unwrap_err();
    assert!(err.to_string().contains("not a UUID"), "{err}");
    let escaped = dir.path().join(format!("{}.tar.gz", "e".repeat(30)));
    assert!(!escaped.exists());
    let archived: i64 = count(
        nb.root(),
        "SELECT is_archived FROM nodes WHERE name = 'Evil'",
    );
    assert_eq!(archived, 0);
}

#[test]
fn a_crafted_canvas_id_stays_inside_the_export() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Hostile");
    let page = page_in(&mut nb, &root);
    let stroke = Stroke {
        id: "s".into(),
        points: vec![
            StrokePoint {
                x: 0.1,
                y: 0.1,
                p: 0.5,
            },
            StrokePoint {
                x: 0.9,
                y: 0.9,
                p: 0.5,
            },
        ],
        color: "#000000".into(),
        width: 2.0,
        tool: "pen".into(),
    };
    let sketch = nb
        .write(|w| w.add_sketch(&page, Frame::PAGE_TEXT, &[stroke], &Settings::new()))
        .unwrap();
    let id = format!("../../../{}", "x".repeat(27));
    let conn = raw(nb.root());
    // Rename the canvas and its content row together, as a crafted file would have them.
    conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
    conn.execute(
        "UPDATE sketch_instances SET instance_id = ?1 WHERE instance_id = ?2",
        [&id, &sketch],
    )
    .unwrap();
    conn.execute(
        "UPDATE canvas_instances SET id = ?1 WHERE id = ?2",
        [&id, &sketch],
    )
    .unwrap();
    drop(conn);
    drop(nb);

    let nb = dunnenote_format::Notebook::open(dir.path().join("Hostile.dunnenote")).unwrap();
    let out = dir.path().join("md");
    let summary = export::to_markdown(&nb, &out, export::Options::default()).unwrap();
    let svg: Vec<_> = summary
        .files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "svg"))
        .collect();
    assert_eq!(svg.len(), 1);
    assert!(svg[0].starts_with("assets"));
    assert_eq!(svg[0].components().count(), 2, "{}", svg[0].display());
    assert!(out.join(svg[0]).is_file());
}

#[test]
fn unsafe_links_export_as_plain_text() {
    let link = |href: &str| {
        json!({"type": "doc", "content": [{"type": "paragraph", "content": [
            {"type": "text", "text": "click",
             "marks": [{"type": "link", "attrs": {"href": href, "title": null}}]}
        ]}]})
    };
    for href in [
        "javascript:alert(1)",
        " JavaScript :alert(1)",
        "data:text/html,x",
        "vbscript:x",
    ] {
        let md = text::markdown(&link(href));
        assert_eq!(md.trim(), "click", "{href}");
    }
    let md = text::markdown(&link("https://example.com/"));
    assert!(md.contains("[click](<https://example.com/>)"), "{md}");
}
