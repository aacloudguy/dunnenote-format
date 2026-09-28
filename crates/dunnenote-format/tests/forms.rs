//! Page settings, forms and captions.

use dunnenote_format::settings_keys::{FormField, Placement};
use dunnenote_format::{payload, At, CanvasKind, Frame, Submission};
use dunnenote_format::{Error, Notebook, Settings};
use serde_json::{json, Value};
use tempfile::TempDir;

#[path = "support/written.rs"]
mod written;
use written::{settings, stroke, PNG};

#[path = "support/common.rs"]
mod common;
use common::{assert_clean, new_notebook, page_in, raw};

fn obj(v: Value) -> dunnenote_format::Settings {
    v.as_object().unwrap().clone()
}

fn stored_page_settings(root: &std::path::Path, page: &str) -> Option<String> {
    raw(root)
        .query_row("SELECT settings FROM nodes WHERE id = ?1", [page], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn page_settings_are_stored_as_dunnenote_stores_them() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Pages");
    let page = page_in(&mut nb, &root);
    let path = nb.root().to_path_buf();
    assert_eq!(stored_page_settings(&path, &page), None);

    // A key DunneNote does not know is kept ahead of its own keys; defaults are not written.
    nb.write(|w| {
        w.merge_page_settings(
            &page,
            &obj(json!({"formFillMode": true, "hideCanvasFrames": false, "x-tool": {"a": 1}})),
        )
    })
    .unwrap();
    assert_eq!(
        stored_page_settings(&path, &page).as_deref(),
        Some(r#"{"x-tool":{"a":1},"formFillMode":true}"#)
    );

    // Removing every key leaves the column NULL again.
    nb.write(|w| w.merge_page_settings(&page, &obj(json!({"formFillMode": null, "x-tool": null}))))
        .unwrap();
    assert_eq!(stored_page_settings(&path, &page), None);

    // Bad values and non-pages are refused.
    let bad = nb.write(|w| w.merge_page_settings(&page, &obj(json!({"formTabOrder": [1, 2]}))));
    assert!(matches!(bad, Err(Error::Invalid(_))), "{bad:?}");
    let not_page = nb.write(|w| w.merge_page_settings(&root, &obj(json!({"formFillMode": true}))));
    assert!(matches!(not_page, Err(Error::Invalid(_))), "{not_page:?}");

    drop(nb);
    let read = Notebook::open(&path).unwrap();
    assert_eq!(read.node(&page).unwrap().settings, None);
    assert_clean(&path);
}

// ---- forms --------------------------------------------------------------------------------------

fn field(name: &str) -> FormField {
    FormField::new(name)
}

fn stamp() -> Submission {
    Submission {
        submitted: Some("2026-01-05T09:30:00+00:00".into()),
        ..Submission::default()
    }
}

/// DunneNote's golden sign-up form, rebuilt: two text fields and a signature, answers table
/// bound by `formDestination`, one submission.
#[test]
fn a_submission_matches_dunnenotes() {
    let golden = Notebook::open(written::fixtures().join("forms.dunnenote")).unwrap();
    let gpage = golden.pages().unwrap().into_iter().next().unwrap();
    let gcanvases = golden.canvases(&gpage.id).unwrap();
    let gtable = gcanvases
        .iter()
        .find(|c| c.kind == CanvasKind::Spreadsheet)
        .unwrap();
    let gdata = golden.dataset(&gtable.id).unwrap().unwrap();
    let gcarrier = gcanvases.iter().find(|c| c.is_hidden()).unwrap();

    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Forms");
    let path = nb.root().to_path_buf();
    let (page, name, team, sig, table) = nb
        .write(|w| {
            let section = w.add_section(&root, "Intake", At::End)?;
            let page = w.add_page(&section, "Sign-up form", At::End)?;
            let s = Settings::new();
            let name = w.add_rich_text(
                &page,
                Frame::new(40, 40, 320, 60),
                Some(&payload::rich_text_from_plain("Ada Example")),
                &s,
            )?;
            let team = w.add_rich_text(
                &page,
                Frame::new(40, 120, 320, 60),
                Some(&payload::rich_text_from_plain("Research")),
                &s,
            )?;
            let sig = w.add_sketch(
                &page,
                Frame::new(40, 200, 320, 120),
                &[stroke(
                    "sig",
                    &[(0.1, 0.5, 0.5), (0.9, 0.4, 0.6)],
                    "#111111",
                    2.0,
                )],
                &s,
            )?;
            let table = w.add_answers_table(&page, Frame::new(420, 40, 480, 200))?;
            let mut required = field("Name");
            required.required = true;
            w.set_form_field(&name, Some(&required))?;
            let mut labelled = field("Team");
            labelled.label = Some("Which team?".into());
            w.set_form_field(&team, Some(&labelled))?;
            w.set_form_field(&sig, Some(&field("Signature")))?;
            w.merge_page_settings(
                &page,
                &settings(json!({
                    "formTabOrder": [name, team, sig],
                    "formDestination": {"kind": "canvas", "id": table}
                })),
            )?;
            Ok((page, name, team, sig, table))
        })
        .unwrap();
    let done = nb.write(|w| w.submit_form(&page, &stamp())).unwrap();
    assert_eq!(done.table, table);
    assert_eq!(done.new_columns, ["c0", "c1", "c2", "c3"]);
    assert_eq!(done.carriers.len(), 1);

    // The page's settings and field roles are stored as DunneNote stores them.
    let ours_page = nb.node(&page).unwrap();
    let norm = |s: &Settings, ids: &[(&str, &str)]| {
        let mut text = Value::Object(s.clone()).to_string();
        for (from, to) in ids {
            text = text.replace(from, to);
        }
        text
    };
    let golden_ids: Vec<String> = gcanvases.iter().map(|c| c.id.clone()).collect();
    assert_eq!(
        norm(
            ours_page.settings.as_ref().unwrap(),
            &[(&name, "A"), (&team, "B"), (&sig, "C"), (&table, "T")]
        ),
        norm(
            gpage.settings.as_ref().unwrap(),
            &[
                (&golden_ids[0], "A"),
                (&golden_ids[1], "B"),
                (&golden_ids[2], "C"),
                (&gtable.id, "T")
            ]
        )
    );
    for (ours, theirs) in [
        (&name, &gcanvases[0]),
        (&team, &gcanvases[1]),
        (&sig, &gcanvases[2]),
    ] {
        assert_eq!(nb.canvas(ours).unwrap().settings, theirs.settings);
    }

    // The answers: same columns, and a row with the same cells (the carrier token aside).
    let data = nb.dataset(&table).unwrap().unwrap();
    let cols = |d: &dunnenote_format::Dataset| -> Vec<(String, String, String, i64)> {
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
            .collect()
    };
    assert_eq!(cols(&data), cols(&gdata));
    let mut cells = data.rows[0].cells.clone();
    let mut gcells = gdata.rows[0].cells.clone();
    assert_eq!(cells["c2"], json!(format!("sketch:{}", done.carriers[0])));
    cells.remove("c2");
    gcells.remove("c2");
    assert_eq!(cells, gcells);

    // The carrier: a hidden sketch in the field's frame and layer, holding its drawing verbatim.
    let carrier = nb.canvas(&done.carriers[0]).unwrap();
    assert_eq!(carrier.kind, CanvasKind::Sketch);
    assert_eq!(carrier.settings, gcarrier.settings);
    assert_eq!(
        (
            carrier.x,
            carrier.y,
            carrier.width,
            carrier.height,
            carrier.z_index
        ),
        (
            gcarrier.x,
            gcarrier.y,
            gcarrier.width,
            gcarrier.height,
            gcarrier.z_index
        )
    );
    let stored = |id: &str| -> String {
        raw(&path)
            .query_row(
                "SELECT data FROM sketch_instances WHERE instance_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(stored(&done.carriers[0]), stored(&sig));
    assert_clean(&path);
}

#[test]
fn a_new_form_submits_and_grows_its_table() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "NewForm");
    let path = nb.root().to_path_buf();
    let (page, table) = nb
        .write(|w| w.add_form(&root, "Untitled form", At::End))
        .unwrap();
    let text = nb.canvases(&page).unwrap()[0].id.clone();
    let t = nb.canvas(&table).unwrap();
    assert_eq!((t.x, t.y, t.width, t.height), (24, 360, 480, 360));
    assert_eq!(
        Value::Object(t.settings.clone()).to_string(),
        r#"{"formTarget":{"ownedByForm":true}}"#
    );
    let d = nb.dataset(&table).unwrap().unwrap();
    assert_eq!(
        (d.source_kind.as_str(), d.columns.len(), d.rows.len()),
        ("csv", 0, 0)
    );
    assert_eq!(
        nb.node(&page)
            .unwrap()
            .settings
            .map(|s| Value::Object(s).to_string()),
        Some(format!(
            r#"{{"formDestination":{{"kind":"canvas","id":"{table}"}}}}"#
        ))
    );

    // No field yet.
    let none = nb.write(|w| w.submit_form(&page, &stamp()));
    assert!(
        matches!(none, Err(Error::Invalid(ref m)) if m.contains("no canvas")),
        "{none:?}"
    );

    // A first answer creates its column and the Submitted column.
    nb.write(|w| {
        w.set_form_field(&text, Some(&field("Comment")))?;
        w.set_rich_text(
            &text,
            &payload::rich_text_from_markdown("# Great\n\nwork **here**"),
        )
    })
    .unwrap();
    let first = nb.write(|w| w.submit_form(&page, &stamp())).unwrap();
    assert_eq!(first.new_columns, ["c0", "c1"]);

    // A second field on a table with rows needs confirming, then adds one column only.
    let pic = nb
        .write(|w| {
            let pic = w.add_picture(&page, (500, 40), None, PNG, &Settings::new())?;
            w.set_form_field(&pic, Some(&field("Photo")))?;
            Ok(pic)
        })
        .unwrap();
    // The answers table moves up a layer: a carrier still takes its field's layer, not the top.
    nb.write(|w| {
        w.transaction().execute(
            "UPDATE canvas_instances SET z_index = 5 WHERE id = ?1",
            [&table],
        )?;
        Ok(())
    })
    .unwrap();
    let unconfirmed = nb.write(|w| w.submit_form(&page, &stamp()));
    assert!(
        matches!(unconfirmed, Err(Error::Invalid(ref m)) if m.contains("Photo")),
        "{unconfirmed:?}"
    );
    let confirmed = Submission {
        confirm_new_columns: true,
        utc_offset_minutes: 90,
        ..Submission::default()
    };
    let second = nb.write(|w| w.submit_form(&page, &confirmed)).unwrap();
    assert_eq!(second.new_columns, ["c2"]);

    let d = nb.dataset(&table).unwrap().unwrap();
    let names: Vec<&str> = d.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Comment", "Submitted", "Photo"]);
    assert_eq!(d.rows[0].cells["c0"], json!("Great work here"));
    assert!(!d.rows[0].cells.contains_key("c2"));
    let stamp2 = d.rows[1].cells["c1"].as_str().unwrap();
    assert!(stamp2.ends_with("+01:30") && stamp2.len() == 25, "{stamp2}");
    let carrier = second.carriers[0].clone();
    assert_eq!(d.rows[1].cells["c2"], json!(format!("picture:{carrier}")));
    let c = nb.canvas(&carrier).unwrap();
    assert!(c.is_hidden());
    assert_eq!(c.z_index, nb.canvas(&pic).unwrap().z_index);
    assert_eq!(c.z_index, 0);
    assert_eq!(c.source_hash, nb.canvas(&pic).unwrap().source_hash);
    assert_clean(&path);
}

#[test]
fn submissions_are_refused_as_dunnenote_refuses_them() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Refusals");
    let (page, table) = nb.write(|w| w.add_form(&root, "F", At::End)).unwrap();
    let text = nb.canvases(&page).unwrap()[0].id.clone();
    let rows = |nb: &Notebook| nb.dataset(&table).unwrap().unwrap().rows.len();

    // Reserved name, table as a field.
    let reserved = nb.write(|w| w.set_form_field(&text, Some(&field(" submitted "))));
    assert!(matches!(reserved, Err(Error::Invalid(_))), "{reserved:?}");
    let table_field = nb.write(|w| w.set_form_field(&table, Some(&field("T"))));
    assert!(
        matches!(table_field, Err(Error::Invalid(_))),
        "{table_field:?}"
    );

    // Required and empty.
    let mut required = field("Name");
    required.required = true;
    nb.write(|w| w.set_form_field(&text, Some(&required)))
        .unwrap();
    let empty = nb.write(|w| w.submit_form(&page, &stamp()));
    assert!(
        matches!(empty, Err(Error::Invalid(ref m)) if m.contains("required")),
        "{empty:?}"
    );

    // Two fields with one name (ignoring case).
    let other = nb
        .write(|w| {
            let t = w.add_rich_text(
                &page,
                Frame::new(400, 40, 200, 60),
                Some(&payload::rich_text_from_plain("x")),
                &Settings::new(),
            )?;
            w.set_form_field(&t, Some(&field("NAME")))?;
            w.set_rich_text(&text, &payload::rich_text_from_plain("Ada"))?;
            Ok(t)
        })
        .unwrap();
    let dup = nb.write(|w| w.submit_form(&page, &stamp()));
    assert!(
        matches!(dup, Err(Error::Invalid(ref m)) if m.contains("both named")),
        "{dup:?}"
    );

    // A hidden field is not part of the form.
    nb.write(|w| w.merge_settings(&other, &settings(json!({"hidden": true}))))
        .unwrap();
    nb.write(|w| w.submit_form(&page, &stamp())).unwrap();
    assert_eq!(rows(&nb), 1);

    // A table the form does not own gets no new columns.
    nb.write(|w| {
        w.merge_settings(&table, &settings(json!({"formTarget": {}})))?;
        w.merge_settings(
            &other,
            &settings(json!({"hidden": null, "formField": {"name": "Other"}})),
        )
    })
    .unwrap();
    let not_owned = nb.write(|w| w.submit_form(&page, &stamp()));
    assert!(
        matches!(not_owned, Err(Error::Invalid(ref m)) if m.contains("Other")),
        "{not_owned:?}"
    );

    // A template's placeholder picture answers nothing: required, it refuses; no carrier is made.
    nb.write(|w| {
        w.merge_settings(&other, &settings(json!({"formField": null})))?;
        let pic = w.add_picture(
            &page,
            (700, 40),
            None,
            PNG,
            &settings(json!({"placeholder": true})),
        )?;
        let mut photo = field("Photo");
        photo.required = true;
        w.set_form_field(&pic, Some(&photo))
    })
    .unwrap();
    let placeholder = nb.write(|w| {
        w.merge_settings(
            &table,
            &settings(json!({"formTarget": {"ownedByForm": true}})),
        )?;
        w.submit_form(
            &page,
            &Submission {
                confirm_new_columns: true,
                ..stamp()
            },
        )
    });
    assert!(
        matches!(placeholder, Err(Error::Invalid(ref m)) if m.contains("Photo")),
        "{placeholder:?}"
    );
    assert_eq!(
        nb.canvases(&page)
            .unwrap()
            .iter()
            .filter(|c| c.is_hidden())
            .count(),
        0,
        "no carrier was made"
    );

    // Answers going to a file are for DunneNote.
    nb.write(|w| {
        w.merge_page_settings(
            &page,
            &settings(json!({"formDestination": {"kind": "file", "format": "csv"}})),
        )
    })
    .unwrap();
    assert!(nb.write(|w| w.submit_form(&page, &stamp())).is_err());
    assert_eq!(rows(&nb), 1, "nothing was appended by a refused submission");
    assert_clean(nb.root());
}

// ---- captions -----------------------------------------------------------------------------------

#[test]
fn a_caption_is_added_as_dunnenote_adds_it() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Captions");
    let page = page_in(&mut nb, &root);
    let (pic, cap, grouped_pic, second) = nb
        .write(|w| {
            let pic = w.add_picture(&page, (100, 50), Some((200, 100)), PNG, &Settings::new())?;
            let cap = w.add_caption(
                &pic,
                Some(&payload::rich_text_from_plain("Figure 1")),
                Placement::Bottom,
            )?;
            let grouped_pic =
                w.add_picture(&page, (400, 50), Some((200, 100)), PNG, &Settings::new())?;
            let other =
                w.add_rich_text(&page, Frame::new(400, 300, 100, 40), None, &Settings::new())?;
            w.add_group(&page, None, &[&grouped_pic, &other])?;
            let second = w.add_caption(&grouped_pic, None, Placement::Movie)?;
            Ok((pic, cap, grouped_pic, second))
        })
        .unwrap();
    let c = nb.canvas(&cap).unwrap();
    assert_eq!((c.x, c.y, c.width, c.height), (100, 102, 200, 48));
    assert_eq!(
        Value::Object(c.settings.clone()).to_string(),
        format!(
            r#"{{"backgroundTransparent":true,"caption":{{"anchor":"{pic}","placement":"bottom"}}}}"#
        )
    );
    let p = nb.canvas(&pic).unwrap();
    assert!(
        c.z_index > p.z_index,
        "a caption sits a layer above its picture"
    );
    assert_eq!(c.group_id, p.group_id);
    assert!(p.group_id.is_some());

    // A picture already in a group keeps it; the caption joins.
    let s = nb.canvas(&second).unwrap();
    assert_eq!(s.group_id, nb.canvas(&grouped_pic).unwrap().group_id);
    assert_eq!((s.x, s.y, s.width, s.height), (430, 78, 140, 48));
    assert_eq!(nb.groups(&page).unwrap().len(), 2);

    let not_picture = nb.write(|w| w.add_caption(&cap, None, Placement::Top));
    assert!(
        matches!(not_picture, Err(Error::Invalid(_))),
        "{not_picture:?}"
    );
    assert_clean(nb.root());
}
