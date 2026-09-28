//! What this library reads about a notebook, in the shape of `fixtures/expected.json` (the shape
//! DunneNote's golden-notebook emitter records). Shared by the golden test and the
//! `emit_written` example.

#![allow(dead_code)]

use std::collections::BTreeSet;

use dunnenote_format::{Canvas, CanvasKind, NodeKind, Notebook};
use serde_json::{json, Map, Value};

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
pub fn observe(nb: &Notebook) -> Value {
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
pub fn first_difference(path: &str, got: &Value, want: &Value) -> Option<String> {
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
