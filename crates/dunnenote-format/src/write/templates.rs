//! Templates: "Make as Template" and "New from Template".
//!
//! A template is an ordinary page with `is_template = 1`. Making one copies the page next to
//! itself as `"<name> (template)"` with its content cleared, except canvases marked
//! `templateKeepContent`; a new page from a template copies everything as it is (`SPEC.md`
//! section 9).

use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

use super::{new_id, At, Writer, PLACEHOLDER_SENTINEL};
use crate::error::{Error, Result};
use crate::settings_keys::{PLACEHOLDER, TEMPLATE_KEEP_CONTENT as TEMPLATE_KEEP_KEY};

/// What a template name ends with.
pub const TEMPLATE_SUFFIX: &str = " (template)";

/// Settings a cleared picture or calendar keeps, in the order they are written after
/// `"placeholder": true`, and which kinds each applies to.
const CLEARED_MEDIA_KEEPS: [(&str, Scope); 14] = [
    ("frameOutlineHidden", Scope::Any),
    ("frameAppearance", Scope::Any),
    ("backgroundTransparent", Scope::Any),
    ("hidden", Scope::Any),
    ("formField", Scope::Any),
    (TEMPLATE_KEEP_KEY, Scope::Any),
    ("renderMode", Scope::Picture),
    ("reflow", Scope::Picture),
    ("sizingMode", Scope::Picture),
    ("rotation", Scope::Picture),
    ("rotationSnapDeg", Scope::Picture),
    ("rotationStepDecimals", Scope::Picture),
    ("scale", Scope::Calendar),
    ("layoutMode", Scope::Calendar),
];

/// Scroll positions, dropped from any other cleared canvas.
const SCROLL_KEYS: [&str; 2] = ["scrollTopPx", "scrollLeftPx"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Any,
    Picture,
    Calendar,
}

fn parse(settings: &str) -> Option<Map<String, Value>> {
    match serde_json::from_str(settings) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

fn keeps_content(settings: &str) -> bool {
    parse(settings).is_some_and(|m| m.get(TEMPLATE_KEEP_KEY) == Some(&Value::Bool(true)))
}

fn cleared_media_settings(kind: &str, settings: &str) -> String {
    let scope = if kind == "calendar" {
        Scope::Calendar
    } else {
        Scope::Picture
    };
    let src = parse(settings).unwrap_or_default();
    let mut out = Map::new();
    out.insert(PLACEHOLDER.into(), Value::Bool(true));
    for (key, key_scope) in CLEARED_MEDIA_KEEPS {
        if key_scope != Scope::Any && key_scope != scope {
            continue;
        }
        if let Some(v) = src.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    Value::Object(out).to_string()
}

fn cleared_layout_settings(settings: &str) -> String {
    match parse(settings) {
        Some(mut m) if SCROLL_KEYS.iter().any(|k| m.contains_key(*k)) => {
            for k in SCROLL_KEYS {
                m.shift_remove(k);
            }
            Value::Object(m).to_string()
        }
        _ => settings.to_string(),
    }
}

struct SourceCanvas {
    id: String,
    kind: String,
    source_hash: String,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    z_index: i64,
    settings: String,
    schema_version: i64,
    group_id: Option<String>,
}

impl Writer<'_> {
    fn require_page(&self, id: &str) -> Result<()> {
        if self.node_kind(id)? != "page" {
            return Err(Error::Invalid(format!("{id} is not a page")));
        }
        Ok(())
    }

    /// Copy every group and canvas of `source` onto the empty page `dest`, as DunneNote does,
    /// clearing content when `clear` (and the canvas does not keep it). Returns old → new ids.
    fn copy_page_canvases(
        &mut self,
        source: &str,
        dest: &str,
        clear: bool,
    ) -> Result<HashMap<String, String>> {
        self.require_page(source)?;
        self.require_page(dest)?;
        let placeholder = if clear {
            Some(self.put_blob(PLACEHOLDER_SENTINEL)?)
        } else {
            None
        };

        // Groups, parents before children.
        let mut pending: Vec<(String, Option<String>, String)> = self
            .tx
            .prepare(
                "SELECT id, parent_group_id, settings FROM groups WHERE page_id = ?1 \
                 ORDER BY created_at, id",
            )?
            .query_map([source], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut groups: HashMap<String, String> = HashMap::new();
        while !pending.is_empty() {
            let before = pending.len();
            let mut still = Vec::new();
            for (id, parent, settings) in pending {
                let parent_new = match &parent {
                    None => None,
                    Some(p) => match groups.get(p) {
                        Some(np) => Some(np.clone()),
                        None => {
                            still.push((id, parent, settings));
                            continue;
                        }
                    },
                };
                let new = new_id();
                self.tx.execute(
                    "INSERT INTO groups(id, page_id, parent_group_id, settings, schema_version, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, unixepoch(), unixepoch())",
                    params![new, dest, parent_new, settings, super::CANVAS_SCHEMA_VERSION],
                )?;
                groups.insert(id, new);
            }
            if still.len() == before {
                return Err(Error::Malformed(format!(
                    "the groups on page {source} do not form a tree"
                )));
            }
            pending = still;
        }

        let canvases: Vec<SourceCanvas> = self
            .tx
            .prepare(
                "SELECT id, kind, source_hash, x, y, width, height, z_index, settings, schema_version, group_id \
                 FROM canvas_instances WHERE page_id = ?1 ORDER BY z_index, z_minor, created_at, id",
            )?
            .query_map([source], |r| {
                Ok(SourceCanvas {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    source_hash: r.get(2)?,
                    x: r.get(3)?,
                    y: r.get(4)?,
                    width: r.get(5)?,
                    height: r.get(6)?,
                    z_index: r.get(7)?,
                    settings: r.get(8)?,
                    schema_version: r.get(9)?,
                    group_id: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut map: HashMap<String, String> = HashMap::new();
        let mut copied: Vec<(String, String)> = Vec::new();
        for c in &canvases {
            let cleared = clear && !keeps_content(&c.settings);
            let media = c.kind == "picture" || c.kind == "calendar";
            let (hash, settings) = if cleared && media {
                (
                    placeholder.clone().expect("made when clearing"),
                    cleared_media_settings(&c.kind, &c.settings),
                )
            } else if cleared {
                (c.source_hash.clone(), cleared_layout_settings(&c.settings))
            } else {
                (c.source_hash.clone(), c.settings.clone())
            };
            let new = new_id();
            self.tx.execute(
                "INSERT INTO canvas_instances(id, page_id, kind, source_hash, x, y, width, height, \
                 z_index, z_minor, settings, schema_version, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
                   (SELECT coalesce(max(z_minor) + 1, 0) FROM canvas_instances WHERE page_id = ?2 AND z_index = ?9), \
                   ?10, ?11, unixepoch(), unixepoch())",
                params![
                    new, dest, c.kind, hash, c.x, c.y, c.width, c.height, c.z_index, settings,
                    c.schema_version
                ],
            )?;
            if let Some(g) = c.group_id.as_ref().and_then(|g| groups.get(g)) {
                self.tx.execute(
                    "UPDATE canvas_instances SET group_id = ?2 WHERE id = ?1",
                    params![new, g],
                )?;
            }
            map.insert(c.id.clone(), new.clone());
            copied.push((new.clone(), settings));

            let content = |table: &str| -> Result<Option<(String, i64)>> {
                Ok(self
                    .tx
                    .query_row(
                        &format!("SELECT data, schema_version FROM {table} WHERE instance_id = ?1"),
                        [&c.id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?)
            };
            let insert = |w: &Self, table: &str, data: &str, version: i64| -> Result<()> {
                w.tx.execute(
                    &format!(
                        "INSERT INTO {table}(instance_id, data, schema_version, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, unixepoch(), unixepoch())"
                    ),
                    params![new, data, version],
                )?;
                Ok(())
            };
            match c.kind.as_str() {
                "rich_text" => match content("rich_text_instances")?.filter(|_| !cleared) {
                    Some((data, version)) => insert(self, "rich_text_instances", &data, version)?,
                    None => insert(
                        self,
                        "rich_text_instances",
                        crate::payload::EMPTY_RICH_TEXT,
                        super::CANVAS_SCHEMA_VERSION,
                    )?,
                },
                "sketch" | "picture" => {
                    let sketch = c.kind == "sketch";
                    match content("sketch_instances")?.filter(|_| !cleared) {
                        Some((data, version)) => insert(self, "sketch_instances", &data, version)?,
                        None if sketch => insert(
                            self,
                            "sketch_instances",
                            crate::payload::EMPTY_SKETCH,
                            super::CANVAS_SCHEMA_VERSION,
                        )?,
                        None => {}
                    }
                }
                "database" | "spreadsheet" => self.clone_dataset(&c.id, &new, cleared)?,
                // A calendar's events are not copied (DunneNote does not copy them either).
                _ => {}
            }
        }
        for (id, settings) in &copied {
            let Some(mut m) = parse(settings) else {
                continue;
            };
            let Some(cap) = m.get_mut("caption").and_then(Value::as_object_mut) else {
                continue;
            };
            let Some(anchor) = cap
                .get("anchor")
                .and_then(Value::as_str)
                .and_then(|a| map.get(a))
            else {
                continue;
            };
            cap.insert("anchor".into(), Value::String(anchor.clone()));
            self.tx.execute(
                "UPDATE canvas_instances SET settings = ?2 WHERE id = ?1",
                params![id, Value::Object(m).to_string()],
            )?;
        }
        Ok(map)
    }

    /// Copy a table's dataset to a new canvas: every row, or (`structure_only`) the same number of
    /// empty rows. Rows are renumbered from 0.
    fn clone_dataset(&mut self, source: &str, dest: &str, structure_only: bool) -> Result<()> {
        let head: Option<(String, String, String)> = self
            .tx
            .query_row(
                "SELECT id, shape, source_kind FROM datasets WHERE instance_id = ?1",
                [source],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((dataset, shape, source_kind)) = head else {
            return Ok(());
        };
        let columns: Vec<(String, String, String, i64)> = self
            .tx
            .prepare(
                "SELECT col_key, name, type_hint, position FROM dataset_columns \
                 WHERE dataset_id = ?1 ORDER BY position, col_key",
            )?
            .query_map([&dataset], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let rows: Vec<String> = self
            .tx
            .prepare("SELECT cells FROM dataset_rows WHERE dataset_id = ?1 ORDER BY seq, id")?
            .query_map([&dataset], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let new = new_id();
        self.tx.execute(
            "INSERT INTO datasets(id, instance_id, shape, source_kind, row_count, schema_version, \
             created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), unixepoch())",
            params![
                new,
                dest,
                shape,
                source_kind,
                rows.len() as i64,
                super::DATASET_SCHEMA_VERSION
            ],
        )?;
        for (key, name, hint, position) in &columns {
            self.tx.execute(
                "INSERT INTO dataset_columns(id, dataset_id, col_key, name, type_hint, position, \
                 created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), unixepoch())",
                params![new_id(), new, key, name, hint, position],
            )?;
        }
        for (seq, cells) in rows.iter().enumerate() {
            // Cells go through serde_json as DunneNote's copy does, so their text is normalised
            // the same way.
            let cells = if structure_only {
                "{}".to_string()
            } else {
                serde_json::from_str::<Value>(cells)
                    .map_err(|e| Error::Malformed(format!("row cells are not JSON: {e}")))?
                    .to_string()
            };
            self.tx.execute(
                "INSERT INTO dataset_rows(id, dataset_id, seq, cells, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, json(?4), unixepoch(), unixepoch())",
                params![new_id(), new, seq as i64, cells],
            )?;
        }
        Ok(())
    }

    /// Copy `source`'s page settings to `dest`, pointing tab order and destination at the copies.
    fn copy_page_settings(
        &mut self,
        source: &str,
        dest: &str,
        map: &HashMap<String, String>,
    ) -> Result<()> {
        let raw: Option<String> =
            self.tx
                .query_row("SELECT settings FROM nodes WHERE id = ?1", [source], |r| {
                    r.get(0)
                })?;
        let Some(raw) = raw else {
            return Ok(());
        };
        let out = match parse(&raw) {
            None => raw,
            Some(mut m) => {
                if let Some(Value::Array(order)) = m.get_mut("formTabOrder") {
                    *order = order
                        .iter()
                        .filter_map(|v| map.get(v.as_str()?).cloned())
                        .map(Value::String)
                        .collect();
                }
                if let Some(Value::Object(d)) = m.get_mut("formDestination") {
                    if d.get("kind").and_then(Value::as_str) == Some("canvas") {
                        if let Some(new) =
                            d.get("id").and_then(Value::as_str).and_then(|i| map.get(i))
                        {
                            d.insert("id".into(), Value::String(new.clone()));
                        }
                    }
                }
                if let Some(new) = m
                    .get("formTargetId")
                    .and_then(Value::as_str)
                    .and_then(|i| map.get(i))
                    .cloned()
                {
                    m.insert("formTargetId".into(), Value::String(new));
                }
                Value::Object(m).to_string()
            }
        };
        if out.len() > super::MAX_SETTINGS_BYTES {
            return Err(Error::Invalid("page settings are too large".into()));
        }
        self.tx.execute(
            "UPDATE nodes SET settings = ?2 WHERE id = ?1",
            params![dest, out],
        )?;
        Ok(())
    }

    /// "Make as Template": add `"<name> (template)"` after the page's last sibling, with
    /// `is_template = 1`, and copy the page into it, clearing every canvas that is not marked
    /// `templateKeepContent`. Returns the template page.
    pub fn make_template(&mut self, page: &str) -> Result<String> {
        self.require_page(page)?;
        let (parent, name): (Option<String>, String) = self.tx.query_row(
            "SELECT parent_id, name FROM nodes WHERE id = ?1",
            [page],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let parent =
            parent.ok_or_else(|| Error::Malformed(format!("page {page} has no parent")))?;
        let template = self.insert_node(
            "page",
            &parent,
            &format!("{name}{TEMPLATE_SUFFIX}"),
            At::End,
        )?;
        self.tx.execute(
            "UPDATE nodes SET is_template = 1 WHERE id = ?1",
            [&template],
        )?;
        let map = self.copy_page_canvases(page, &template, true)?;
        self.copy_page_settings(page, &template, &map)?;
        Ok(template)
    }

    /// "New from Template": a page at the end of `parent`, named after the template without
    /// `" (template)"`, with a full copy of the template's canvases and settings.
    pub fn new_from_template(&mut self, template: &str, parent: &str) -> Result<String> {
        self.require_page(template)?;
        let name: String =
            self.tx
                .query_row("SELECT name FROM nodes WHERE id = ?1", [template], |r| {
                    r.get(0)
                })?;
        let name = name
            .strip_suffix(TEMPLATE_SUFFIX)
            .map_or_else(|| name.clone(), str::to_string);
        let page = self.insert_node("page", parent, &name, At::End)?;
        let map = self.copy_page_canvases(template, &page, false)?;
        self.copy_page_settings(template, &page, &map)?;
        Ok(page)
    }
}
