//! Archive and retrieve, tags and metadata — against DunneNote's golden `archive` and `tags`
//! notebooks.

use dunnenote_format::write::{read_snapshot, snapshot_bytes, ARCHIVE_DIR};
use dunnenote_format::{ArchiveReason, At, Error, Frame, MetaValue, Notebook, Settings};
use tempfile::TempDir;

#[path = "support/common.rs"]
mod common;
use common::{assert_clean, count, new_notebook, page_in, raw};

#[path = "support/written.rs"]
mod written;
use written::{copy_dir, fixtures, PNG};

fn golden_snapshot() -> (String, Vec<u8>) {
    let dir = fixtures().join("archive.dunnenote").join(ARCHIVE_DIR);
    let entry = std::fs::read_dir(dir).unwrap().next().unwrap().unwrap();
    let name = entry.file_name().to_string_lossy().into_owned();
    (
        name.trim_end_matches(".tar.gz").to_string(),
        std::fs::read(entry.path()).unwrap(),
    )
}

#[test]
fn snapshots_are_byte_for_byte_dunnenotes() {
    let (id, bytes) = golden_snapshot();
    let nodes = read_snapshot(&bytes).unwrap();
    assert_eq!(nodes[0].id, id);
    assert_eq!(snapshot_bytes(&nodes, &id).unwrap(), bytes);
}

#[test]
fn archiving_a_page_as_dunnenote_does() {
    let (id, golden_bytes) = golden_snapshot();
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("archive.dunnenote");
    copy_dir(&fixtures().join("archive.dunnenote"), &root).unwrap();
    let sidecar = root.join(ARCHIVE_DIR).join(format!("{id}.tar.gz"));
    let mut nb = Notebook::open_writable(&root).unwrap();
    let before = nb.node(&id).unwrap();
    assert_eq!(before.archive_reason.as_deref(), Some("superseded"));

    // Retrieve: columns cleared, the (valid) snapshot deleted, canvases untouched.
    nb.write(|w| w.retrieve_node(&id)).unwrap();
    let n = nb.node(&id).unwrap();
    assert!(!n.is_archived && n.archive_reason.is_none() && n.archived_at.is_none());
    assert!(!sidecar.exists());

    // Archive again: the snapshot holds the subtree as it was, then the row is marked.
    nb.write(|w| w.archive_node(&id, ArchiveReason::Other, Some("  merged into v2 ")))
        .unwrap();
    let n = nb.node(&id).unwrap();
    assert!(n.is_archived);
    assert_eq!(n.archive_reason.as_deref(), Some("other"));
    assert_eq!(n.archive_note.as_deref(), Some("merged into v2"));
    let snap = read_snapshot(&std::fs::read(&sidecar).unwrap()).unwrap();
    let golden = read_snapshot(&golden_bytes).unwrap();
    assert_eq!(snap.len(), golden.len());
    assert_eq!(
        (
            snap[0].id.as_str(),
            snap[0].name.as_str(),
            snap[0].is_archived
        ),
        (golden[0].id.as_str(), golden[0].name.as_str(), false)
    );
    for c in nb.canvases(&id).unwrap() {
        assert_eq!(
            c.lifecycle, "active",
            "archiving a page leaves its canvases alone"
        );
    }

    // Already archived; a page under it; a note with the wrong reason.
    let again = nb.write(|w| w.archive_node(&id, ArchiveReason::Wrong, None));
    assert!(matches!(again, Err(Error::Invalid(_))), "{again:?}");
    let noted = nb.write(|w| {
        w.archive_node(
            &before.parent_id.clone().unwrap(),
            ArchiveReason::Wrong,
            Some("why"),
        )
    });
    assert!(matches!(noted, Err(Error::Invalid(_))), "{noted:?}");
    let section = before.parent_id.clone().unwrap();
    let blocked = nb.write(|w| w.archive_node(&section, ArchiveReason::Irrelevant, None));
    assert!(
        matches!(blocked, Err(Error::Invalid(ref m)) if m.contains("already archived")),
        "{blocked:?}"
    );
    // The notebook itself can be archived around an archived page.
    let nb_root = nb.notebook_node().unwrap().id;
    nb.write(|w| w.archive_node(&nb_root, ArchiveReason::Superseded, None))
        .unwrap();
    assert!(root
        .join(ARCHIVE_DIR)
        .join(format!("{nb_root}.tar.gz"))
        .is_file());
    // Only the archived row changes: the section under the notebook is untouched.
    let s = nb.node(&section).unwrap();
    assert!(!s.is_archived && s.archive_reason.is_none());
    assert!(
        nb.node(&id).unwrap().is_archived,
        "the page keeps its own archive"
    );
    drop(nb);
    assert_clean(&root);
}

#[test]
fn canvases_archive_and_retrieve() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Canvases");
    let page = page_in(&mut nb, &root);
    let c = nb
        .write(|w| w.add_rich_text(&page, Frame::PAGE_TEXT, None, &Settings::new()))
        .unwrap();
    nb.write(|w| w.archive_canvas(&c, ArchiveReason::Other, Some("old")))
        .unwrap();
    let a = nb.canvas(&c).unwrap();
    assert_eq!(
        (
            a.lifecycle.as_str(),
            a.lifecycle_reason.as_deref(),
            a.lifecycle_note.as_deref()
        ),
        ("archived", Some("other"), Some("old"))
    );
    assert!(a.lifecycle_at.is_some());
    assert!(nb
        .write(|w| w.archive_canvas(&c, ArchiveReason::Wrong, None))
        .is_err());
    nb.write(|w| w.retrieve_canvas(&c)).unwrap();
    let r = nb.canvas(&c).unwrap();
    assert_eq!(r.lifecycle, "active");
    assert!(r.lifecycle_reason.is_none() && r.lifecycle_note.is_none() && r.lifecycle_at.is_none());
    assert!(nb.write(|w| w.retrieve_canvas(&c)).is_err());
    assert!(nb.write(|w| w.retrieve_node(&page)).is_err());
    assert_clean(nb.root());
}

/// DunneNote's golden tags notebook, rebuilt: the same tags, folds, aliases, applications and
/// metadata rows.
#[test]
fn tags_and_metadata_match_dunnenotes() {
    let golden = Notebook::open(fixtures().join("tags.dunnenote")).unwrap();
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Tags");
    let path = nb.root().to_path_buf();
    let csv = golden
        .read_blob("5c587aa5b73a3dbc0abd8b8f2fb6bb2b41cee2dfa94c0b9a1ec4bdf04d1b4b4e")
        .or_else(|_| {
            let page = golden.pages()?.into_iter().next().unwrap();
            let t = golden
                .canvases(&page.id)?
                .into_iter()
                .find(|c| c.kind.as_str() == "database")
                .unwrap();
            golden.read_blob(&t.source_hash)
        })
        .unwrap();
    nb.write(|w| {
        let section = w.add_section(&root, "Field notes", At::End)?;
        let page = w.add_page(&section, "Site visit", At::End)?;
        let text = w.add_rich_text(&page, Frame::new(40, 40, 400, 120), None, &Settings::new())?;
        let pic = w.add_picture(&page, (480, 40), Some((160, 160)), PNG, &Settings::new())?;
        let table =
            w.add_table_from_csv(&page, Frame::new(40, 200, 420, 180), &csv, &Settings::new())?;
        let (dataset, row): (String, String) = w.transaction().query_row(
            "SELECT d.id, r.id FROM datasets d JOIN dataset_rows r ON r.dataset_id = d.id \
             WHERE d.instance_id = ?1 ORDER BY r.seq LIMIT 1",
            [&table],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let cafe = w.tag("Café")?;
        w.add_tag_alias(&cafe, "Coffee")?;
        let alpha = w.tag("Project Alpha")?;
        w.add_tag_alias(&alpha, "alpha")?;
        let urgent = w.tag("Urgent")?;
        assert_eq!(
            w.tag(" CAFE ")?,
            cafe,
            "a name folding to an existing tag finds it"
        );
        assert_eq!(w.tag("coffee")?, cafe, "and so does an alias");
        w.apply_tag(&alpha, "node", &page)?;
        w.apply_tag(&cafe, "instance", &text)?;
        w.apply_tag(&alpha, "instance", &pic)?;
        assert!(
            !w.apply_tag(&alpha, "instance", &pic)?,
            "applying twice is a no-op"
        );
        w.apply_tag(&urgent, "dataset", &dataset)?;
        w.apply_tag(&urgent, "dataset_row", &row)?;
        w.set_meta(
            "node",
            &page,
            "Vendor",
            &MetaValue::Text(" Example Surveys ".into()),
            "user",
        )?;
        w.set_meta(
            "instance",
            &pic,
            "capture_time",
            &MetaValue::Datetime {
                epoch_secs: 1_767_600_000.0,
                iso8601: "2026-01-05T08:00:00Z".into(),
            },
            "user",
        )?;
        w.set_meta(
            "instance",
            &pic,
            "geo",
            &MetaValue::Geo {
                lat: 51.5007,
                lon: -0.1246,
            },
            "user",
        )?;
        w.set_meta(
            "instance",
            &pic,
            "place",
            &MetaValue::Text("Westminster".into()),
            "user",
        )?;
        Ok(())
    })
    .unwrap();

    let tags = |nb: &Notebook| -> Vec<(String, String, Vec<String>)> {
        nb.tags()
            .unwrap()
            .into_iter()
            .map(|t| (t.name, t.name_folded, t.aliases))
            .collect()
    };
    assert_eq!(tags(&nb), tags(&golden));
    let applied = |root: &std::path::Path| -> Vec<(String, String)> {
        raw(root)
            .prepare(
                "SELECT it.source_kind, t.name FROM item_tags it JOIN tags t ON t.id = it.tag_id \
                 ORDER BY it.source_kind, t.name",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    assert_eq!(applied(&path), applied(&fixtures().join("tags.dunnenote")));
    type MetaRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<f64>,
        String,
    );
    let meta =
        |root: &std::path::Path| -> Vec<MetaRow> {
            raw(root)
            .prepare(
                "SELECT source_kind, key, value_text, value_folded, value_num, value_num2, source \
                 FROM item_meta ORDER BY source_kind, key",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
        };
    assert_eq!(meta(&path), meta(&fixtures().join("tags.dunnenote")));
    assert_clean(&path);
}

#[test]
fn tag_names_aliases_merge_and_rename() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Merge");
    let page = page_in(&mut nb, &root);
    let path = nb.root().to_path_buf();
    let (a, b) = nb
        .write(|w| {
            let a = w.tag("Alpha")?;
            let b = w.tag("Beta")?;
            w.add_tag_alias(&a, "A1")?;
            w.add_tag_alias(&b, "alpha-old")?;
            w.apply_tag(&a, "node", &page)?;
            w.apply_tag(&b, "node", &page)?;
            Ok((a, b))
        })
        .unwrap();

    // Clashes across names and aliases are refused.
    for (tag, alias) in [(&a, "beta"), (&a, "ALPHA-OLD"), (&b, "a1")] {
        let r = nb.write(|w| w.add_tag_alias(tag, alias));
        assert!(matches!(r, Err(Error::Invalid(_))), "{alias}: {r:?}");
    }
    assert!(nb.write(|w| w.rename_tag(&a, "BETA")).is_err());
    assert!(nb.write(|w| w.rename_tag(&a, "alpha-old")).is_err());
    // Renaming to its own alias takes the alias's place.
    nb.write(|w| w.rename_tag(&a, "a1")).unwrap();
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM tag_aliases WHERE alias_folded = 'a1'"
        ),
        0
    );
    // Unknown kinds and missing items are refused.
    assert!(nb.write(|w| w.apply_tag(&a, "canvas", &page)).is_err());
    assert!(nb
        .write(|w| w.apply_tag(&a, "node", "01a0e92f-0000-7000-8000-000000000000"))
        .is_err());
    assert!(nb
        .write(|w| w.set_meta(
            "node",
            &page,
            "place",
            &MetaValue::Geo { lat: 1.0, lon: 1.0 },
            "user"
        ))
        .is_err());
    assert!(nb
        .write(|w| w.set_meta(
            "node",
            &page,
            "custom",
            &MetaValue::Geo { lat: 1.0, lon: 1.0 },
            "user"
        ))
        .is_err());
    assert!(nb
        .write(|w| w.set_meta(
            "node",
            &page,
            "geo",
            &MetaValue::Geo {
                lat: 91.0,
                lon: 1.0
            },
            "user"
        ))
        .is_err());

    // Merge B into A: applications collapse, B's aliases move, B's name becomes an alias.
    nb.write(|w| w.merge_tags(&b, &a)).unwrap();
    let t = nb.tags().unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].aliases, ["alpha-old", "Beta"]);
    assert_eq!(count(&path, "SELECT count(*) FROM item_tags"), 1);
    assert!(nb.write(|w| w.merge_tags(&a, &a)).is_err());
    nb.write(|w| w.delete_tag(&a)).unwrap();
    assert_eq!(count(&path, "SELECT count(*) FROM item_tags"), 0);
    assert_eq!(count(&path, "SELECT count(*) FROM tag_aliases"), 0);
    assert_clean(&path);
}
