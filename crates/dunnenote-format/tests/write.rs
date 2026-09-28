//! Creating and changing notebooks.
//!
//! What this library writes must look, row for row, like what DunneNote writes: the golden
//! notebooks in `fixtures/` (written by DunneNote itself) are the reference for sentinel blobs,
//! schema versions, layering and the manifest. DunneNote's own round-trip test separately opens
//! the notebooks built in `support/written.rs` in the app.

use dunnenote_format::{payload, At, CanvasKind, Error, Frame, Notebook, Settings};
use serde_json::{json, Value};
use tempfile::TempDir;

#[path = "support/written.rs"]
mod written;
use written::{settings, stroke, PNG};

#[path = "support/common.rs"]
mod common;
use common::{assert_clean, count, new_notebook, page_in, raw};

// ---- creating ---------------------------------------------------------------------------------

#[test]
fn a_new_notebook_is_laid_out_as_dunnenote_lays_it_out() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("Field Trip.dunnenote");
    let nb = Notebook::create(&root, None).unwrap();

    // format.json: pretty-printed, no trailing newline, fields in DunneNote's order.
    let text = std::fs::read_to_string(root.join("format.json")).unwrap();
    assert!(!text.ends_with('\n'));
    let m = nb.manifest();
    let want = format!(
        "{{\n  \"format\": \"dunnenote\",\n  \"format_version\": \"0.18.0\",\n  \"notebook_id\": \"{}\",\n  \"created_at\": {},\n  \"created_by\": \"dunnenote-format/{}\"\n}}",
        m.notebook_id,
        m.created_at,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(text, want);
    assert_eq!(&m.notebook_id[14..15], "7", "ids are UUIDv7");

    // The root node is named after the folder, at the first position, and is not the manifest id.
    let node = nb.notebook_node().unwrap();
    assert_eq!(node.name, "Field Trip");
    assert_eq!(node.position, "80");
    assert_ne!(node.id, m.notebook_id);
    assert!(root.join(".dunnenote.lock").is_file());
    assert_eq!(count(&root, "PRAGMA user_version"), 18);
    assert_eq!(count(&root, "SELECT count(*) FROM blobs"), 0);
    drop(nb);
    assert!(
        root.join(".dunnenote.lock").is_file(),
        "the lock file is never deleted"
    );
    assert_clean(&root);
}

#[test]
fn create_refuses_bad_or_existing_paths() {
    let dir = TempDir::new().unwrap();
    assert!(matches!(
        Notebook::create(dir.path().join("notes"), None),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        Notebook::create(dir.path().join(".dunnenote"), None),
        Err(Error::Invalid(_))
    ));
    let root = dir.path().join("A.dunnenote");
    Notebook::create(&root, None).unwrap();
    assert!(matches!(
        Notebook::create(&root, None),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        Notebook::create(dir.path().join("B.dunnenote"), Some("bad\nname")),
        Err(Error::Invalid(_))
    ));
    assert!(
        !dir.path().join("B.dunnenote").exists(),
        "a failed create leaves nothing"
    );
}

// ---- the lock and the version gate ------------------------------------------------------------

#[test]
fn the_lock_keeps_out_a_second_writer_but_not_readers() {
    let dir = TempDir::new().unwrap();
    let (nb, _) = new_notebook(&dir, "Locked");
    let root = nb.root().to_path_buf();
    assert!(matches!(Notebook::open_writable(&root), Err(Error::Locked)));
    let reader = Notebook::open(&root).unwrap();
    assert!(reader.locked_by_another_process().unwrap());
    assert!(!nb.locked_by_another_process().unwrap());
    drop(nb);
    assert!(!reader.locked_by_another_process().unwrap());
    Notebook::open_writable(&root).unwrap();
}

#[test]
fn only_schema_18_is_written() {
    let dir = TempDir::new().unwrap();
    let (nb, _) = new_notebook(&dir, "Future");
    let root = nb.root().to_path_buf();
    drop(nb);
    raw(&root).pragma_update(None, "user_version", 19).unwrap();
    assert!(Notebook::open(&root).is_ok(), "19 still reads");
    assert!(matches!(
        Notebook::open_writable(&root),
        Err(Error::ReadOnly(_))
    ));
    raw(&root).pragma_update(None, "user_version", 17).unwrap();
    assert!(matches!(
        Notebook::open_writable(&root),
        Err(Error::SchemaTooOld { .. })
    ));
}

#[test]
fn a_read_only_handle_cannot_write() {
    let dir = TempDir::new().unwrap();
    let (nb, _) = new_notebook(&dir, "RO");
    let root = nb.root().to_path_buf();
    drop(nb);
    let mut reader = Notebook::open(&root).unwrap();
    assert!(matches!(reader.write(|_| Ok(())), Err(Error::ReadOnly(_))));
}

#[test]
fn a_damaged_notebook_is_refused() {
    let dir = TempDir::new().unwrap();
    let (nb, _) = new_notebook(&dir, "Damaged");
    let root = nb.root().to_path_buf();
    drop(nb);
    let c = raw(&root);
    c.pragma_update(None, "foreign_keys", "OFF").unwrap();
    c.execute(
        "INSERT INTO groups(id, page_id, settings, schema_version, created_at, updated_at) \
         VALUES ('01920000-0000-7000-8000-0000000000ff', '01920000-0000-7000-8000-000000000999', '{}', 1, 1, 1)",
        [],
    )
    .unwrap();
    drop(c);
    assert!(matches!(
        Notebook::open_writable(&root),
        Err(Error::Refused(_))
    ));
}

// ---- the page tree ----------------------------------------------------------------------------

#[test]
fn siblings_go_where_they_are_asked() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Order");
    let section = nb
        .write(|w| {
            let s = w.add_section(&root, "S", At::End)?;
            let b = w.add_page(&s, "b", At::End)?;
            let d = w.add_page(&s, "d", At::End)?;
            w.add_page(&s, "a", At::Start)?;
            w.add_page(&s, "c", At::After(&b))?;
            w.add_page(&s, "e", At::After(&d))?;
            w.add_page(&s, "c2", At::Before(&d))?;
            Ok(s)
        })
        .unwrap();
    let names: Vec<String> = nb
        .children(&section)
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect();
    assert_eq!(names, ["a", "b", "c", "c2", "d", "e"]);
    let positions: Vec<String> = nb
        .children(&section)
        .unwrap()
        .into_iter()
        .map(|n| n.position)
        .collect();
    assert_eq!(
        positions[1], "80",
        "the first child of an empty parent is 80"
    );
    assert_eq!(nb.node(&section).unwrap().child_count, 6);
}

#[test]
fn pages_are_leaves_and_names_are_checked() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Rules");
    let page = page_in(&mut nb, &root);
    assert!(nb
        .write(|w| w.add_page(&page, "under a page", At::End))
        .is_err());
    assert!(nb.write(|w| w.add_section(&root, "", At::End)).is_err());
    assert!(nb
        .write(|w| w.add_section(&root, &"x".repeat(256), At::End))
        .is_err());
    assert!(nb
        .write(|w| w.add_page(&root, "not a sibling", At::After(&page)))
        .is_err());
    nb.write(|w| w.rename(&page, "Renamed  (kept as typed) "))
        .unwrap();
    assert_eq!(nb.node(&page).unwrap().name, "Renamed  (kept as typed) ");
}

#[test]
fn nothing_is_added_under_an_archived_section() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Archived");
    let section = nb.write(|w| w.add_section(&root, "Old", At::End)).unwrap();
    let path = nb.root().to_path_buf();
    drop(nb);
    raw(&path)
        .execute(
            "UPDATE nodes SET is_archived = 1, archive_reason = 'superseded', archived_at = unixepoch() WHERE id = ?1",
            [&section],
        )
        .unwrap();
    let mut nb = Notebook::open_writable(&path).unwrap();
    assert!(nb.write(|w| w.add_page(&section, "P", At::End)).is_err());
}

// ---- canvases ---------------------------------------------------------------------------------

#[test]
fn canvases_match_what_dunnenote_writes() {
    // The reference: the rich text, sketch and picture rows in DunneNote's own every-kind notebook.
    let golden = Notebook::open(written::fixtures().join("every-kind.dunnenote")).unwrap();
    let reference = |kind: CanvasKind| {
        golden
            .pages()
            .unwrap()
            .iter()
            .flat_map(|p| golden.canvases(&p.id).unwrap())
            .find(|c| c.kind == kind)
            .unwrap()
    };

    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Canvases");
    let page = page_in(&mut nb, &root);
    let (text, sketch, picture) = nb
        .write(|w| {
            Ok((
                w.add_rich_text(&page, Frame::PAGE_TEXT, None, &Settings::new())?,
                w.add_sketch(&page, Frame::new(0, 0, 100, 100), &[], &Settings::new())?,
                w.add_picture(&page, (5, 6), None, PNG, &Settings::new())?,
            ))
        })
        .unwrap();

    for (id, kind) in [
        (&text, CanvasKind::RichText),
        (&sketch, CanvasKind::Sketch),
        (&picture, CanvasKind::Picture),
    ] {
        let ours = nb.canvas(id).unwrap();
        let theirs = reference(kind);
        assert_eq!(ours.kind, kind);
        assert_eq!(ours.source_hash, theirs.source_hash, "{kind:?} source blob");
        assert_eq!(ours.schema_version, theirs.schema_version);
        assert_eq!(ours.lifecycle, "active");
        assert_eq!(ours.group_id, None);
    }
    assert_eq!(
        nb.rich_text(&text).unwrap().unwrap().doc,
        serde_json::from_str::<Value>(payload::EMPTY_RICH_TEXT).unwrap()
    );
    assert_eq!(
        nb.sketch(&sketch).unwrap().unwrap().data,
        json!({"v": 1, "strokes": []})
    );
    // Picture frame is the image's own size; no markup until some is drawn.
    let pic = nb.canvas(&picture).unwrap();
    assert_eq!((pic.x, pic.y, pic.width, pic.height), (5, 6, 16, 16));
    assert!(nb.sketch(&picture).unwrap().is_none());

    // Layering: one major layer, minors in creation order.
    let layers: Vec<(i64, i64)> = nb
        .canvases(&page)
        .unwrap()
        .iter()
        .map(|c| (c.z_index, c.z_minor))
        .collect();
    assert_eq!(layers, [(0, 0), (0, 1), (0, 2)]);

    // Reference counts are the triggers' business and come out right.
    let path = nb.root().to_path_buf();
    assert_eq!(
        count(&path, "SELECT count(*) FROM blobs WHERE refcount <> 1"),
        0
    );
    assert_clean(&path);
}

#[test]
fn new_canvases_join_the_top_layer() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Layers");
    let page = page_in(&mut nb, &root);
    let first = nb
        .write(|w| w.add_rich_text(&page, Frame::PAGE_TEXT, None, &Settings::new()))
        .unwrap();
    let path = nb.root().to_path_buf();
    drop(nb);
    raw(&path)
        .execute(
            "UPDATE canvas_instances SET z_index = 3 WHERE id = ?1",
            [&first],
        )
        .unwrap();
    let mut nb = Notebook::open_writable(&path).unwrap();
    let second = nb
        .write(|w| w.add_sketch(&page, Frame::new(0, 0, 10, 10), &[], &Settings::new()))
        .unwrap();
    let c = nb.canvas(&second).unwrap();
    assert_eq!((c.z_index, c.z_minor), (3, 1));
}

#[test]
fn a_failed_write_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Atomic");
    let bad_doc = json!({"type": "doc", "content": [{"type": "table"}]});
    let err = nb
        .write(|w| {
            let s = w.add_section(&root, "S", At::End)?;
            let p = w.add_page(&s, "P", At::End)?;
            w.add_rich_text(&p, Frame::PAGE_TEXT, Some(&bad_doc), &Settings::new())
        })
        .unwrap_err();
    assert!(err.to_string().contains("\"table\""), "{err}");
    assert_eq!(nb.walk().unwrap().len(), 1, "only the root remains");
    assert_eq!(count(nb.root(), "SELECT count(*) FROM blobs"), 0);
}

#[test]
fn payloads_are_validated() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Payloads");
    let page = page_in(&mut nb, &root);
    assert!(nb
        .write(|w| w.add_sketch(
            &page,
            Frame::new(0, 0, 10, 10),
            &[stroke("x", &[(0.1, 0.1, 0.5)], "#111111", 2.0)],
            &Settings::new()
        ))
        .is_err());
    assert!(nb
        .write(|w| w.add_picture(
            &page,
            (0, 0),
            None,
            b"GIF87a but not really",
            &Settings::new()
        ))
        .is_err());
    assert!(nb
        .write(|w| w.add_rich_text(&page, Frame::new(0, 0, 0, 10), None, &Settings::new()))
        .is_err());
    let text = nb
        .write(|w| w.add_rich_text(&page, Frame::PAGE_TEXT, None, &Settings::new()))
        .unwrap();
    assert!(
        nb.write(|w| w.set_sketch(&text, &[])).is_err(),
        "kind is checked"
    );
    assert!(nb.write(|w| w.set_markup(&text, &[])).is_err());
    let huge = settings(json!({"blob": "x".repeat(70 * 1024)}));
    assert!(nb.write(|w| w.merge_settings(&text, &huge)).is_err());
}

#[test]
fn settings_merge_keeps_what_it_does_not_know() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Merge");
    let page = page_in(&mut nb, &root);
    let id = nb
        .write(|w| {
            w.add_rich_text(
                &page,
                Frame::PAGE_TEXT,
                None,
                &settings(json!({"zeta": 1, "hidden": true, "frameOutlineHidden": true})),
            )
        })
        .unwrap();
    nb.write(|w| {
        w.merge_settings(
            &id,
            &settings(json!({"hidden": null, "zeta": 2, "backgroundTransparent": true})),
        )
    })
    .unwrap();
    let stored: String = raw(nb.root())
        .query_row(
            "SELECT settings FROM canvas_instances WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        stored,
        r#"{"zeta":2,"frameOutlineHidden":true,"backgroundTransparent":true}"#
    );
}

#[test]
fn rich_text_and_strokes_round_trip() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Round");
    let page = page_in(&mut nb, &root);
    let doc = written::every_node_doc("01920000-0000-7000-8000-00000000000a");
    let (text, pic) = nb
        .write(|w| {
            let t = w.add_rich_text(&page, Frame::PAGE_TEXT, Some(&doc), &Settings::new())?;
            let p = w.add_picture(&page, (0, 0), Some((64, 64)), PNG, &Settings::new())?;
            w.set_markup(
                &p,
                &[stroke(
                    "m",
                    &[(0.0, 0.0, 1.0), (1.0, 1.0, 1.0)],
                    "#111111",
                    2.0,
                )],
            )?;
            Ok((t, p))
        })
        .unwrap();
    assert_eq!(nb.rich_text(&text).unwrap().unwrap().doc, doc);
    let markup = nb.sketch(&pic).unwrap().unwrap();
    assert_eq!(markup.strokes().len(), 1);
    assert_eq!(markup.strokes()[0].points[1].x, 1.0);
    // A second markup write replaces the first (one markup layer per picture).
    nb.write(|w| w.set_markup(&pic, &[])).unwrap();
    assert!(nb.sketch(&pic).unwrap().unwrap().strokes().is_empty());
    assert_eq!(nb.canvas(&pic).unwrap().width, 64);
}

#[test]
fn a_shared_picture_is_stored_once() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Dedupe");
    let page = page_in(&mut nb, &root);
    nb.write(|w| {
        w.add_picture(&page, (0, 0), None, PNG, &Settings::new())?;
        w.add_picture(&page, (20, 0), None, PNG, &Settings::new())
    })
    .unwrap();
    nb.write(|w| w.add_picture(&page, (40, 0), None, PNG, &Settings::new()))
        .unwrap();
    let path = nb.root();
    assert_eq!(
        count(path, "SELECT refcount FROM blobs WHERE size_bytes = 95"),
        3
    );
    assert_clean(path);
}

// ---- groups -----------------------------------------------------------------------------------

#[test]
fn groups_nest_on_one_page() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Groups");
    let page = page_in(&mut nb, &root);
    let other = nb
        .write(|w| {
            let s = w.add_section(&root, "Other", At::End)?;
            w.add_page(&s, "Other page", At::End)
        })
        .unwrap();
    let (a, b, far) = nb
        .write(|w| {
            Ok((
                w.add_sketch(&page, Frame::new(0, 0, 10, 10), &[], &Settings::new())?,
                w.add_sketch(&page, Frame::new(0, 0, 10, 10), &[], &Settings::new())?,
                w.add_sketch(&other, Frame::new(0, 0, 10, 10), &[], &Settings::new())?,
            ))
        })
        .unwrap();
    let (outer, inner) = nb
        .write(|w| {
            let outer = w.add_group(&page, None, &[&a])?;
            let inner = w.add_group(&page, Some(&outer), &[&b])?;
            Ok((outer, inner))
        })
        .unwrap();
    assert_eq!(
        nb.canvas(&a).unwrap().group_id.as_deref(),
        Some(outer.as_str())
    );
    assert_eq!(
        nb.canvas(&b).unwrap().group_id.as_deref(),
        Some(inner.as_str())
    );
    let groups = nb.groups(&page).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[1].parent_group_id.as_deref(), Some(outer.as_str()));

    assert!(nb.write(|w| w.add_group(&page, None, &[&far])).is_err());
    assert!(nb
        .write(|w| w.add_group(&other, Some(&outer), &[]))
        .is_err());
    nb.write(|w| w.set_group(&[&b], None)).unwrap();
    assert_eq!(nb.canvas(&b).unwrap().group_id, None);

    // Depth limit.
    let err = nb.write(|w| {
        let mut parent = inner.clone();
        for _ in 0..64 {
            parent = w.add_group(&page, Some(&parent), &[])?;
        }
        Ok(())
    });
    assert!(err.is_err());
}

// ---- the search cache -------------------------------------------------------------------------

#[test]
fn writing_empties_the_search_index_so_dunnenote_rebuilds_it() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("golden.dunnenote");
    written::copy_dir(&written::fixtures().join("every-kind.dunnenote"), &root).unwrap();
    assert!(count(&root, "SELECT count(*) FROM search_index") > 0);
    assert!(count(&root, "SELECT count(*) FROM search_index_fts") > 0);
    let mut nb = Notebook::open_writable(&root).unwrap();
    let parent = nb.notebook_node().unwrap().id;
    nb.write(|w| w.add_section(&parent, "New", At::End))
        .unwrap();
    drop(nb);
    assert_eq!(count(&root, "SELECT count(*) FROM search_index"), 0);
    assert_eq!(count(&root, "SELECT count(*) FROM search_index_fts"), 0);
    assert_eq!(count(&root, "SELECT count(*) FROM search_index_trgm"), 0);
}

// ---- the conformance notebooks ----------------------------------------------------------------

#[test]
fn every_written_notebook_verifies_clean_and_reads_back() {
    let dir = TempDir::new().unwrap();
    let built = written::build_all(dir.path()).unwrap();
    assert_eq!(built.len(), 5);
    for (name, root) in &built {
        assert_clean(root);
        assert_eq!(
            count(root, "SELECT count(*) FROM search_index"),
            0,
            "{name}"
        );
        let nb = Notebook::open(root).unwrap();
        assert!(
            !nb.locked_by_another_process().unwrap(),
            "{name}: lock released"
        );
    }

    let path_of = |name: &str| &built.iter().find(|(n, _)| *n == name).unwrap().1;
    let every = Notebook::open(path_of("written-every-kind")).unwrap();
    let kinds: Vec<CanvasKind> = every
        .pages()
        .unwrap()
        .iter()
        .flat_map(|p| every.canvases(&p.id).unwrap())
        .map(|c| c.kind)
        .collect();
    for kind in [
        CanvasKind::RichText,
        CanvasKind::Sketch,
        CanvasKind::Picture,
    ] {
        assert!(kinds.contains(&kind), "every-kind has a {kind:?}");
    }
    let names: Vec<String> = every
        .walk()
        .unwrap()
        .into_iter()
        .map(|(_, n)| n.name)
        .collect();
    assert_eq!(
        names,
        [
            "Written Every Kind",
            "Archive later",
            "Research",
            "Overview",
            "After overview",
            "Between",
            "Sources",
            "Background",
            "Reading list",
            "Plans",
            "Schedule"
        ]
    );

    // Writing into DunneNote's notebook kept everything it had.
    let golden = Notebook::open(written::fixtures().join("every-kind.dunnenote")).unwrap();
    let changed = Notebook::open(path_of("written-into-golden")).unwrap();
    let before = golden.counts().unwrap();
    let after = changed.counts().unwrap();
    assert_eq!(after.pages, before.pages + 1);
    assert_eq!(after.canvases, before.canvases + 2);
    assert_eq!(after.tags, before.tags);
    for page in golden.pages().unwrap() {
        for c in golden.canvases(&page.id).unwrap() {
            let now = changed.canvas(&c.id).unwrap();
            assert_eq!(now.settings, c.settings, "canvas {} settings kept", c.id);
        }
    }
}

#[test]
fn the_spec_lists_the_sentinels_this_library_writes() {
    use dunnenote_format::write::{
        PLACEHOLDER_SENTINEL, RICH_TEXT_SENTINEL, SKETCH_SENTINEL, SPREADSHEET_SENTINEL,
    };
    use sha2::{Digest, Sha256};
    let spec = std::fs::read_to_string(written::fixtures().join("../SPEC.md")).unwrap();
    for bytes in [
        RICH_TEXT_SENTINEL,
        SKETCH_SENTINEL,
        SPREADSHEET_SENTINEL,
        PLACEHOLDER_SENTINEL,
    ] {
        let text = std::str::from_utf8(bytes).unwrap();
        let hash = hex::encode(Sha256::digest(bytes));
        let row = format!("| `{text}` | `{hash}` |");
        assert!(spec.contains(&row), "SPEC.md is missing {row}");
    }
}
