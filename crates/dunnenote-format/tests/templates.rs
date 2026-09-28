//! Templates, against DunneNote's golden `template` notebook: making a template of its source
//! page again, and making a page from its template, must give what DunneNote gave.

use std::path::Path;

use dunnenote_format::settings_keys::{FormField, Placement};
use dunnenote_format::{payload, Canvas, Frame, Notebook, Settings};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use tempfile::TempDir;

#[path = "support/common.rs"]
mod common;
use common::{assert_clean, new_notebook, page_in, raw};

#[path = "support/written.rs"]
mod written;
use written::{copy_dir, fixtures, settings, stroke, PNG};

/// A page's canvases as comparable facts: kind, source hash, frame, layer, settings (with the
/// page's own canvas ids replaced by their position) and stored content.
fn page_shape(root: &Path, nb: &Notebook, page: &str) -> Vec<Value> {
    let canvases: Vec<Canvas> = nb.canvases(page).unwrap();
    let ids: Vec<String> = canvases.iter().map(|c| c.id.clone()).collect();
    let conn = raw(root);
    let content = |table: &str, id: &str| -> Option<String> {
        conn.query_row(
            &format!("SELECT data FROM {table} WHERE instance_id = ?1"),
            [id],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
    };
    canvases
        .iter()
        .map(|c| {
            let mut settings = Value::Object(c.settings.clone()).to_string();
            for (i, id) in ids.iter().enumerate() {
                settings = settings.replace(id.as_str(), &format!("#{i}"));
            }
            let table = nb.dataset(&c.id).unwrap().map(|d| {
                json!({
                    "shape": d.shape, "source_kind": d.source_kind, "row_count": d.row_count,
                    "columns": d.columns.iter().map(|c| json!([c.col_key, c.name, c.type_hint, c.position])).collect::<Vec<_>>(),
                    "rows": d.rows.iter().map(|r| json!([r.seq, r.cells])).collect::<Vec<_>>(),
                })
            });
            json!({
                "kind": c.kind.as_str(),
                "source_hash": c.source_hash,
                "frame": [c.x, c.y, c.width, c.height],
                "layer": [c.z_index, c.z_minor],
                "group": c.group_id.is_some(),
                "settings": settings,
                "rich_text": content("rich_text_instances", &c.id),
                "sketch": content("sketch_instances", &c.id),
                "table": table,
            })
        })
        .collect()
}

#[test]
fn templates_match_dunnenotes() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("template.dunnenote");
    copy_dir(&fixtures().join("template.dunnenote"), &root).unwrap();
    let golden = Notebook::open(fixtures().join("template.dunnenote")).unwrap();
    let pages = golden.pages().unwrap();
    let source = pages
        .iter()
        .find(|p| p.name == "Meeting notes" && !p.is_template)
        .unwrap();
    let template = pages.iter().find(|p| p.is_template).unwrap();
    let made = pages
        .iter()
        .find(|p| p.name == "Meeting notes" && p.id != source.id)
        .unwrap();

    let mut nb = Notebook::open_writable(&root).unwrap();
    let (ours_template, ours_page) = nb
        .write(|w| {
            let t = w.make_template(&source.id)?;
            let p = w.new_from_template(&template.id, template.parent_id.as_deref().unwrap())?;
            Ok((t, p))
        })
        .unwrap();

    let t = nb.node(&ours_template).unwrap();
    assert_eq!(t.name, "Meeting notes (template)");
    assert!(t.is_template);
    assert_eq!(t.parent_id, source.parent_id);
    let p = nb.node(&ours_page).unwrap();
    assert_eq!((p.name.as_str(), p.is_template), ("Meeting notes", false));

    let golden_root = fixtures().join("template.dunnenote");
    assert_eq!(
        page_shape(&root, &nb, &ours_template),
        page_shape(&golden_root, &golden, &template.id)
    );
    assert_eq!(
        page_shape(&root, &nb, &ours_page),
        page_shape(&golden_root, &golden, &made.id)
    );
    drop(nb);
    assert_clean(&root);
}

#[test]
fn every_kind_is_cleared_or_kept_and_references_follow_the_copies() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Everything");
    let path = nb.root().to_path_buf();
    let page = page_in(&mut nb, &root);
    let (text, pic, table, form_table) = nb
        .write(|w| {
            let text = w.add_rich_text(
                &page,
                Frame::PAGE_TEXT,
                Some(&payload::rich_text_from_plain("Agenda")),
                &settings(json!({"scrollTopPx": 40, "frameOutlineHidden": true})),
            )?;
            let sketch = w.add_sketch(
                &page,
                Frame::new(40, 400, 200, 100),
                &[stroke("s", &[(0.1, 0.1, 0.5), (0.9, 0.9, 0.5)], "#111111", 2.0)],
                &settings(json!({"templateKeepContent": true})),
            )?;
            let pic = w.add_picture(
                &page,
                (300, 400),
                Some((100, 100)),
                PNG,
                &settings(json!({"alt": "x", "rotation": 90, "formField": {"name": "Photo"}, "hotspots": []})),
            )?;
            w.set_markup(&pic, &[stroke("m", &[(0.0, 0.0, 0.5), (1.0, 1.0, 0.5)], "#ff0000", 2.0)])?;
            w.add_caption(&pic, Some(&payload::rich_text_from_plain("Photo")), Placement::Bottom)?;
            let table = w.add_table(&page, Frame::new(40, 600, 300, 200), &Settings::new())?;
            let rows: Vec<String> = w
                .transaction()
                .prepare("SELECT r.id FROM dataset_rows r JOIN datasets d ON d.id = r.dataset_id WHERE d.instance_id = ?1 ORDER BY seq")?
                .query_map([&table], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            w.set_cell(&table, &rows[1], "c0", &json!("filled"))?;
            w.delete_row(&table, &rows[0])?;
            let form_table = w.add_answers_table(&page, Frame::new(400, 600, 300, 200))?;
            w.set_form_field(&text, Some(&FormField::new("Agenda")))?;
            w.merge_page_settings(
                &page,
                &settings(json!({
                    "formTabOrder": [text, sketch, "01a0e92f-dead-7000-8000-000000000000"],
                    "formDestination": {"kind": "canvas", "id": form_table},
                    "x-other": 1
                })),
            )?;
            Ok((text, pic, table, form_table))
        })
        .unwrap();

    let (template, made) = nb
        .write(|w| {
            let t = w.make_template(&page)?;
            let parent: String = w.transaction().query_row(
                "SELECT parent_id FROM nodes WHERE id = ?1",
                [&page],
                |r| r.get(0),
            )?;
            let p = w.new_from_template(&t, &parent)?;
            Ok((t, p))
        })
        .unwrap();

    let copies = nb.canvases(&template).unwrap();
    let by_kind = |k: &str| {
        copies
            .iter()
            .filter(|c| c.kind.as_str() == k)
            .collect::<Vec<_>>()
    };
    let placeholder = "f999f31fe5e877e891b7f85f4d26559818b8f3f01604900ed6073b9e284eaac4";

    // Picture: the placeholder, kept keys only (placeholder first), no markup.
    let tpic = by_kind("picture")[0];
    assert_eq!(tpic.source_hash, placeholder);
    assert_eq!(
        Value::Object(tpic.settings.clone()).to_string(),
        r#"{"placeholder":true,"formField":{"name":"Photo"},"rotation":90}"#
    );
    assert!(nb.sketch(&tpic.id).unwrap().is_none());
    // Text: emptied, scroll position dropped, other keys kept in place.
    let ttexts = by_kind("rich_text");
    let ttext = ttexts
        .iter()
        .find(|c| c.settings.contains_key("formField"))
        .unwrap();
    assert_eq!(
        Value::Object(ttext.settings.clone()).to_string(),
        r#"{"frameOutlineHidden":true,"formField":{"name":"Agenda"}}"#
    );
    assert_eq!(
        nb.rich_text(&ttext.id).unwrap().unwrap().doc,
        serde_json::from_str::<Value>(payload::EMPTY_RICH_TEXT).unwrap()
    );
    // The caption follows the copied picture, in a copied group.
    let tcap = ttexts
        .iter()
        .find(|c| c.settings.contains_key("caption"))
        .unwrap();
    assert_eq!(tcap.settings["caption"]["anchor"], json!(tpic.id));
    assert_eq!(tcap.group_id, tpic.group_id);
    assert!(tpic.group_id.is_some());
    assert_ne!(tpic.group_id, nb.canvas(&pic).unwrap().group_id);
    // Sketch marked templateKeepContent keeps its strokes.
    assert_eq!(
        nb.sketch(&by_kind("sketch")[0].id)
            .unwrap()
            .unwrap()
            .strokes()
            .len(),
        1
    );
    // Tables keep columns and row count, not values; rows renumbered.
    let tdata = nb.dataset(&by_kind("spreadsheet")[0].id).unwrap().unwrap();
    let odata = nb.dataset(&table).unwrap().unwrap();
    assert_eq!(tdata.columns.len(), odata.columns.len());
    assert_eq!(tdata.rows.iter().map(|r| r.seq).collect::<Vec<_>>(), [0, 1]);
    assert!(tdata.rows.iter().all(|r| r.cells.is_empty()));
    // Page settings point at the copies; stale ids are dropped.
    let ts = nb.node(&template).unwrap().settings.unwrap();
    let tform = by_kind("spreadsheet")
        .into_iter()
        .find(|c| c.settings.contains_key("formTarget"))
        .unwrap()
        .id
        .clone();
    assert_eq!(ts["formDestination"]["id"], json!(tform));
    assert_eq!(ts["formTabOrder"].as_array().unwrap().len(), 2);
    assert_eq!(ts["formTabOrder"][0], json!(ttext.id));
    assert_eq!(ts["x-other"], json!(1));
    assert_ne!(tform, form_table);

    // A page from the template copies it as it is, placeholder included.
    let made_pic = nb
        .canvases(&made)
        .unwrap()
        .into_iter()
        .find(|c| c.kind.as_str() == "picture")
        .unwrap();
    assert_eq!(made_pic.source_hash, placeholder);
    assert_eq!(nb.node(&made).unwrap().name, "P");
    let _ = text;
    drop(nb);
    assert_clean(&path);
}
