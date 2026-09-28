//! Page settings, forms and captions.

use dunnenote_format::{Error, Notebook};
use serde_json::{json, Value};
use tempfile::TempDir;

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
