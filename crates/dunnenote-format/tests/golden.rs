//! Conformance against notebooks written by DunneNote itself.
//!
//! `fixtures/*.dunnenote` were produced by DunneNote's own code (its golden-notebook emitter
//! drives the same functions the app's interface calls), and `fixtures/expected.json` records
//! what DunneNote's own read paths report about each one. This library must report the same
//! thing, field for field. See `fixtures/README.md`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use dunnenote_format::{export, verify, Canvas, CanvasKind, NodeKind, Notebook, VerifyLevel};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn expected() -> Map<String, Value> {
    let text = std::fs::read_to_string(fixtures().join("expected.json")).expect("expected.json");
    let v: Value = serde_json::from_str(&text).expect("expected.json parses");
    v["notebooks"].as_object().expect("notebooks").clone()
}

fn open(name: &str) -> Notebook {
    Notebook::open(fixtures().join(format!("{name}.dunnenote")))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The golden notebooks on disk, by name.
fn on_disk() -> BTreeSet<String> {
    std::fs::read_dir(fixtures())
        .expect("fixtures dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".dunnenote").map(str::to_string)
        })
        .collect()
}

// ---- what this library reads, in the shape of expected.json ------------------------------------

fn content(nb: &Notebook, c: &Canvas) -> Value {
    match c.kind {
        CanvasKind::RichText => json!({"doc": nb.rich_text(&c.id).unwrap().map(|r| r.doc)}),
        CanvasKind::Sketch | CanvasKind::Picture => {
            json!({"strokes": nb.sketch(&c.id).unwrap().map(|s| s.data)})
        }
        CanvasKind::Database | CanvasKind::Spreadsheet => {
            let ds = nb
                .dataset(&c.id)
                .unwrap()
                .expect("a table canvas has a dataset");
            json!({
                "dataset_id": ds.id,
                "shape": ds.shape,
                "source_kind": ds.source_kind,
                "row_count": ds.row_count,
                "columns": ds.columns.iter().map(|c| json!({
                    "col_key": c.col_key, "name": c.name, "type_hint": c.type_hint,
                    "position": c.position,
                })).collect::<Vec<_>>(),
                "rows": ds.rows.iter().map(|r| json!({"id": r.id, "seq": r.seq, "cells": r.cells})).collect::<Vec<_>>(),
            })
        }
        CanvasKind::Calendar => {
            json!({"events": nb.calendar_events(&c.id).unwrap().iter().map(|e| json!({
            "id": e.id, "uid": e.uid, "summary": e.summary, "location": e.location,
            "description": e.description, "start_utc": e.start_utc, "end_utc": e.end_utc,
            "all_day": e.all_day, "tzid": e.tzid,
        })).collect::<Vec<_>>()})
        }
    }
}

fn tag_entry(nb: &Notebook, kind: &str, id: &str, out: &mut Vec<Value>) {
    let names: Vec<String> = nb
        .tags_of(kind, id)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    if !names.is_empty() {
        out.push(json!({"source_kind": kind, "source_id": id, "tags": names}));
    }
}

fn meta_entry(nb: &Notebook, kind: &str, id: &str, out: &mut Vec<Value>) {
    for m in nb.meta_of(kind, id).unwrap() {
        out.push(json!({
            "source_kind": kind, "source_id": id, "key": m.key,
            "value_text": m.value_text, "value_num": m.value_num, "value_num2": m.value_num2,
            "source": m.source,
        }));
    }
}

/// Everything this library reads about a notebook, in the same order DunneNote reports it.
fn observe(nb: &Notebook) -> Value {
    let mut tree = Vec::new();
    let mut page_ids = Vec::new();
    for (depth, node) in nb.walk().unwrap() {
        tree.push(json!({
            "depth": depth,
            "kind": node.kind.as_str(),
            "id": node.id,
            "name": node.name,
            "is_template": node.is_template,
            "is_archived": node.is_archived,
            "settings": node.settings,
        }));
        if node.kind == NodeKind::Page {
            page_ids.push(node.id);
        }
    }

    let mut item_tags = Vec::new();
    let mut meta = Vec::new();
    for node in &tree {
        let id = node["id"].as_str().unwrap();
        tag_entry(nb, "node", id, &mut item_tags);
        meta_entry(nb, "node", id, &mut meta);
    }

    let mut pages = Map::new();
    for page in &page_ids {
        let mut canvases = Vec::new();
        for c in nb.canvases(page).unwrap() {
            let body = content(nb, &c);
            for kind in ["canvas", "instance"] {
                tag_entry(nb, kind, &c.id, &mut item_tags);
                meta_entry(nb, kind, &c.id, &mut meta);
            }
            if let Some(rows) = body.get("rows").and_then(Value::as_array) {
                tag_entry(
                    nb,
                    "dataset",
                    body["dataset_id"].as_str().unwrap(),
                    &mut item_tags,
                );
                for r in rows {
                    tag_entry(nb, "dataset_row", r["id"].as_str().unwrap(), &mut item_tags);
                }
            }
            canvases.push(json!({
                "id": c.id, "kind": c.kind.as_str(), "source_hash": c.source_hash,
                "x": c.x, "y": c.y, "width": c.width, "height": c.height,
                "z_index": c.z_index, "z_minor": c.z_minor,
                "group_id": c.group_id,
                "lifecycle": c.lifecycle, "lifecycle_reason": c.lifecycle_reason,
                "lifecycle_note": c.lifecycle_note,
                "settings": c.settings,
                "content": body,
            }));
        }
        let groups: Vec<Value> = nb
            .groups(page)
            .unwrap()
            .into_iter()
            .map(|g| json!({"id": g.id, "parent_group_id": g.parent_group_id}))
            .collect();
        pages.insert(
            page.clone(),
            json!({"canvases": canvases, "groups": groups}),
        );
    }

    let tags: Vec<Value> = nb
        .tags()
        .unwrap()
        .into_iter()
        .map(|t| json!({"id": t.id, "name": t.name, "name_folded": t.name_folded, "aliases": t.aliases}))
        .collect();

    json!({"tree": tree, "pages": pages, "tags": tags, "item_tags": item_tags, "meta": meta})
}

/// The first place two JSON values differ, as a path, so a failure names the field.
fn first_difference(path: &str, got: &Value, want: &Value) -> Option<String> {
    match (got, want) {
        (Value::Object(g), Value::Object(w)) => {
            let keys: BTreeSet<&String> = g.keys().chain(w.keys()).collect();
            keys.into_iter().find_map(|k| match (g.get(k), w.get(k)) {
                (Some(a), Some(b)) => first_difference(&format!("{path}.{k}"), a, b),
                (a, b) => Some(format!("{path}.{k}: library {a:?}, DunneNote {b:?}")),
            })
        }
        (Value::Array(g), Value::Array(w)) => {
            if g.len() != w.len() {
                return Some(format!(
                    "{path}: library has {} items, DunneNote {}",
                    g.len(),
                    w.len()
                ));
            }
            g.iter()
                .zip(w)
                .enumerate()
                .find_map(|(i, (a, b))| first_difference(&format!("{path}[{i}]"), a, b))
        }
        _ if got == want => None,
        _ => Some(format!("{path}: library {got}, DunneNote {want}")),
    }
}

// ---- the tests ----------------------------------------------------------------------------------

#[test]
fn every_golden_notebook_has_an_expectation() {
    let want: BTreeSet<String> = expected().keys().cloned().collect();
    assert_eq!(
        on_disk(),
        want,
        "fixtures on disk and expected.json disagree"
    );
    assert!(want.len() >= 6, "the golden set shrank: {want:?}");
}

#[test]
fn the_library_reads_each_golden_notebook_as_dunnenote_does() {
    for (name, want) in expected() {
        let got = observe(&open(&name));
        if let Some(diff) = first_difference(&name, &got, &want) {
            panic!("{diff}");
        }
    }
}

#[test]
fn golden_notebooks_cover_every_canvas_kind_and_role() {
    let all = expected();
    let canvases: Vec<&Value> = all
        .values()
        .flat_map(|nb| nb["pages"].as_object().unwrap().values())
        .flat_map(|p| p["canvases"].as_array().unwrap())
        .collect();
    for kind in CanvasKind::ALL {
        assert!(
            canvases.iter().any(|c| c["kind"] == kind.as_str()),
            "no golden canvas of kind {}",
            kind.as_str()
        );
    }
    let has = |key: &str| canvases.iter().any(|c| c["settings"].get(key).is_some());
    for key in [
        "caption",
        "formField",
        "formTarget",
        "hidden",
        "templateKeepContent",
        "placeholder",
    ] {
        assert!(has(key), "no golden canvas carries settings.{key}");
    }
    assert!(canvases.iter().any(|c| c["lifecycle"] == "archived"));
    assert!(canvases
        .iter()
        .any(|c| c["kind"] == "picture" && !c["content"]["strokes"].is_null()));
    assert!(canvases.iter().any(|c| !c["group_id"].is_null()));
    let tree: Vec<&Value> = all
        .values()
        .flat_map(|nb| nb["tree"].as_array().unwrap())
        .collect();
    assert!(tree.iter().any(|n| n["is_template"] == true));
    assert!(tree.iter().any(|n| n["is_archived"] == true));
    let kinds: BTreeSet<&str> = all
        .values()
        .flat_map(|nb| nb["item_tags"].as_array().unwrap())
        .map(|t| t["source_kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        BTreeSet::from(["dataset", "dataset_row", "instance", "node"])
    );
}

#[test]
fn golden_notebooks_verify_clean() {
    for name in on_disk() {
        let report = verify(&open(&name), VerifyLevel::Full).unwrap();
        assert!(report.is_clean(), "{name}: {:?}", report.findings);
    }
}

#[test]
fn json_export_carries_everything_the_app_reads() {
    for (name, want) in expected() {
        let doc = export::to_json(&open(&name)).unwrap();
        assert_eq!(doc["export_version"], export::JSON_EXPORT_VERSION);
        let text = doc.to_string();
        // Every node and canvas id DunneNote reports appears in the export.
        for node in want["tree"].as_array().unwrap() {
            assert!(
                text.contains(node["id"].as_str().unwrap()),
                "{name}: node missing"
            );
        }
        for page in want["pages"].as_object().unwrap().values() {
            for c in page["canvases"].as_array().unwrap() {
                assert!(
                    text.contains(c["id"].as_str().unwrap()),
                    "{name}: canvas missing"
                );
            }
        }
        // And it round-trips through serde as a document, not just a string.
        let back: Value = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        assert_eq!(back, doc);
    }
}

fn read_tree(dir: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(s) = std::fs::read_to_string(&p) {
                out.push_str(&s);
            }
        }
    }
    out
}

#[test]
fn markdown_and_csv_exports_carry_the_visible_text() {
    let tmp = TempDir::new().unwrap();
    let nb = open("every-kind");
    let md = tmp.path().join("md");
    let summary = export::to_markdown(&nb, &md, export::Options::default()).unwrap();
    assert_eq!(summary.pages, 3);
    let text = read_tree(&md);
    for needle in [
        "## Project overview",
        "**one canvas of every kind**",
        "[web link](<https://example.com/>)",
        "| Draft the outline | Ada | 3 | true |",
        "| R3C1 | R3C2 | R3C3 |",
        "**Weekly stand-up**",
        "![Four coloured squares](<../assets/",
        "<path d=\"M30.00 40.00",
    ] {
        assert!(text.contains(needle), "markdown export lacks {needle:?}");
    }
    assert!(
        export::to_markdown(&nb, &md, export::Options::default()).is_err(),
        "a second export into the same folder must refuse, not overwrite"
    );

    let csv = tmp.path().join("csv");
    let summary = export::to_csv(&nb, &csv, export::Options::default()).unwrap();
    assert_eq!((summary.tables, summary.calendars), (2, 1));
    let text = read_tree(&csv);
    assert!(text.contains("Task,Owner,Hours,Done\r\nDraft the outline,Ada,3,true\r\n"));
    assert!(text.contains("Weekly stand-up,2026-01-05T15:00:00Z,2026-01-05T15:30:00Z,false,Room 4"));
}

#[test]
fn exports_skip_archived_and_hidden_content_unless_asked() {
    let tmp = TempDir::new().unwrap();
    let archive = open("archive");
    let plain = read_tree(&{
        let d = tmp.path().join("a");
        export::to_markdown(&archive, &d, export::Options::default()).unwrap();
        d
    });
    assert!(plain.contains("The paragraph that stays."));
    assert!(!plain.contains("An abandoned paragraph."));
    assert!(!plain.contains("Replaced by the second draft."));
    let all = read_tree(&{
        let d = tmp.path().join("b");
        export::to_markdown(
            &archive,
            &d,
            export::Options {
                include_archived: true,
            },
        )
        .unwrap();
        d
    });
    assert!(all.contains("An abandoned paragraph."));
    assert!(all.contains("Replaced by the second draft."));

    // The form's hidden signature carrier is not drawn; the answer token stays in the table.
    let forms = read_tree(&{
        let d = tmp.path().join("f");
        export::to_markdown(&open("forms"), &d, export::Options::default()).unwrap();
        d
    });
    assert_eq!(
        forms.matches("![Sketch]").count(),
        1,
        "hidden carrier was exported"
    );
    assert!(forms.contains("sketch:"));
}
