//! Forms and captions.
//!
//! A form is a page whose canvases carry `formField` roles and whose settings name where the
//! answers go (`formDestination`). Submitting reads what each field canvas currently shows and
//! appends one row to the answers table, as DunneNote's Submit button does (`SPEC.md` section 9).

use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

use super::{At, Frame, Writer, SPREADSHEET_SENTINEL};
use crate::error::{Error, Result};
use crate::ingest::TypeHint;
use crate::model::{CanvasKind, Settings};
use crate::settings_keys::{
    self, is_true, read_form_destination, FormDestination, FormField, Placement,
    BACKGROUND_TRANSPARENT, CAPTION, FORM_FIELD, FORM_TARGET, HIDDEN, PLACEHOLDER,
};
use crate::text::{rfc3339_offset, utc_date};

/// The reserved answers column every submission stamps with its date and time.
pub const SUBMITTED_COLUMN: &str = "Submitted";

/// Where DunneNote puts a new form's answers table, and its size.
pub const FORM_TABLE_FRAME: Frame = Frame {
    x: 24,
    y: 360,
    width: 480,
    height: 360,
};

/// A caption's height when it is added.
pub const CAPTION_HEIGHT: i64 = 48;
const CAPTION_INSET: i64 = 8;
const CAPTION_MOVIE_BOTTOM_INSET: i64 = 24;

/// Frame of a carrier whose source canvas has gone.
const CARRIER_FALLBACK: Frame = Frame {
    x: 0,
    y: 0,
    width: 360,
    height: 300,
};

/// How a submission is made.
#[derive(Debug, Clone, Default)]
pub struct Submission {
    /// The `Submitted` value. Without it, the current time in the zone `utc_offset_minutes`.
    pub submitted: Option<String>,
    /// The submitter's zone, in minutes east of UTC. It dates the `Submitted` stamp and turns a
    /// calendar field's displayed day into its `YYYY-MM-DD` answer.
    pub utc_offset_minutes: i32,
    /// Allow adding columns to an answers table that already has rows (DunneNote asks first).
    pub confirm_new_columns: bool,
}

/// What a submission wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submitted {
    /// The answers table (an Editable table canvas).
    pub table: String,
    /// The new row.
    pub row: String,
    /// Keys of columns the submission added, in order.
    pub new_columns: Vec<String>,
    /// Hidden canvases created to hold drawn and picture answers.
    pub carriers: Vec<String>,
}

/// A field canvas as the submit reads it.
struct FieldCanvas {
    id: String,
    kind: CanvasKind,
    source_hash: String,
    frame: Frame,
    z_index: i64,
    settings: Settings,
    role: FormField,
}

enum Answer {
    Text(String),
    Sketch { doc: String },
    Picture { hash: String },
}

/// Is `c` whitespace in JavaScript's `\s`?
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// A rich-text answer as DunneNote reads it: every text node's text, a space after each node that
/// has content, runs of whitespace collapsed to one space, trimmed. (Number atoms and notebook
/// links have no `text`, so they give nothing.)
pub fn answer_text(doc: &Value) -> String {
    fn walk(node: &Value, out: &mut String) {
        match node {
            Value::Array(items) => items.iter().for_each(|n| walk(n, out)),
            Value::Object(rec) => {
                if let Some(Value::String(t)) = rec.get("text") {
                    out.push_str(t);
                }
                if let Some(Value::Array(children)) = rec.get("content") {
                    children.iter().for_each(|c| walk(c, out));
                    out.push(' ');
                }
            }
            _ => {}
        }
    }
    let mut raw = String::new();
    walk(doc, &mut raw);
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;
    for c in raw.chars() {
        if js_space(c) {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
        }
    }
    out
}

/// The rectangle DunneNote gives a caption over `anchor` for `placement` (`None` for `user`).
pub fn caption_frame(anchor: Frame, height: i64, placement: Placement) -> Option<Frame> {
    let (ax, ay) = (anchor.x, anchor.y);
    let (aw, ah) = (anchor.width.max(1), anchor.height.max(1));
    let h = height.min(ah);
    let round = |v: f64| v.round() as i64;
    let corner_w = round(aw as f64 * 0.45).max(1).min(aw);
    let movie_w = round(aw as f64 * 0.7).max(1).min(aw);
    let clamp_x = |v: i64, w: i64| v.max(ax).min(ax + aw - w);
    let clamp_y = |v: i64| v.max(ay).min(ay + ah - h);
    let i = CAPTION_INSET;
    Some(match placement {
        Placement::Bottom => Frame::new(ax, ay + ah - h, aw, h),
        Placement::Top => Frame::new(ax, ay, aw, h),
        Placement::CornerTopLeft => {
            Frame::new(clamp_x(ax + i, corner_w), clamp_y(ay + i), corner_w, h)
        }
        Placement::CornerTopRight => Frame::new(
            clamp_x(ax + aw - corner_w - i, corner_w),
            clamp_y(ay + i),
            corner_w,
            h,
        ),
        Placement::CornerBottomLeft => Frame::new(
            clamp_x(ax + i, corner_w),
            clamp_y(ay + ah - h - i),
            corner_w,
            h,
        ),
        Placement::CornerBottomRight => Frame::new(
            clamp_x(ax + aw - corner_w - i, corner_w),
            clamp_y(ay + ah - h - i),
            corner_w,
            h,
        ),
        Placement::Movie => {
            // JavaScript's Math.round: halves round up.
            let x = ax + ((aw - movie_w) as f64 / 2.0 + 0.5).floor() as i64;
            let y = ay + ah - h - CAPTION_MOVIE_BOTTOM_INSET;
            Frame::new(clamp_x(x, movie_w), clamp_y(y), movie_w, h)
        }
        Placement::User => return None,
    })
}

impl Writer<'_> {
    fn canvas_settings(&self, canvas: &str) -> Result<Settings> {
        let stored: String = self.tx.query_row(
            "SELECT settings FROM canvas_instances WHERE id = ?1",
            [canvas],
            |r| r.get(0),
        )?;
        match serde_json::from_str(&stored) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Ok(Settings::new()),
        }
    }

    fn page_settings(&self, page: &str) -> Result<Settings> {
        let stored: Option<String> =
            self.tx
                .query_row("SELECT settings FROM nodes WHERE id = ?1", [page], |r| {
                    r.get(0)
                })?;
        Ok(match stored.map(|s| serde_json::from_str(&s)) {
            Some(Ok(Value::Object(map))) => map,
            _ => Settings::new(),
        })
    }

    /// Make a canvas a form field, or (with `None`) stop it being one. Tables cannot be fields.
    pub fn set_form_field(&mut self, canvas: &str, field: Option<&FormField>) -> Result<()> {
        let (kind, _) = self.canvas_kind(canvas)?;
        let value = match field {
            None => Value::Null,
            Some(_) if matches!(kind, CanvasKind::Database | CanvasKind::Spreadsheet) => {
                return Err(Error::Invalid(
                    "a table holds many values and an answer holds one, so a table can't be a form field"
                        .into(),
                ))
            }
            Some(f) => {
                if f.name.trim().eq_ignore_ascii_case(SUBMITTED_COLUMN) {
                    return Err(Error::Invalid(format!(
                        "\"{SUBMITTED_COLUMN}\" can't be a field name: every submission records its time in that column"
                    )));
                }
                f.to_value()?
            }
        };
        let mut patch = Settings::new();
        patch.insert(FORM_FIELD.into(), value);
        self.merge_settings(canvas, &patch)?;
        Ok(())
    }

    /// Create a form as DunneNote's "New ▸ Form" does: a page with its text box and an empty
    /// answers table the form owns, bound as the page's destination. Returns `(page, table)`.
    pub fn add_form(&mut self, parent: &str, name: &str, at: At<'_>) -> Result<(String, String)> {
        let page = self.add_page(parent, name, at)?;
        self.add_rich_text(&page, Frame::PAGE_TEXT, None, &Settings::new())?;
        let table = self.add_answers_table(&page, FORM_TABLE_FRAME)?;
        let mut patch = Settings::new();
        patch.insert(
            "formDestination".into(),
            FormDestination::Canvas(table.clone()).to_value(),
        );
        self.merge_page_settings(&page, &patch)?;
        Ok((page, table))
    }

    /// Add an empty answers table (no columns, no rows) that its form owns.
    pub fn add_answers_table(&mut self, page: &str, frame: Frame) -> Result<String> {
        let hash = self.put_blob(SPREADSHEET_SENTINEL)?;
        let mut settings = Settings::new();
        settings.insert(FORM_TARGET.into(), settings_keys::form_target(true));
        let id = self.insert_canvas(page, CanvasKind::Spreadsheet, &hash, frame, &settings)?;
        self.insert_dataset(&id, "csv", &[], &[])?;
        Ok(id)
    }

    /// The page's form fields in the order a submit reads them: the ids in `formTabOrder` that
    /// are still on the page, then the rest in layer order. Hidden and archived canvases, and
    /// canvases without a usable `formField`, are left out.
    pub fn form_fields(&self, page: &str) -> Result<Vec<String>> {
        Ok(self
            .field_canvases(page)?
            .into_iter()
            .map(|f| f.id)
            .collect())
    }

    fn field_canvases(&self, page: &str) -> Result<Vec<FieldCanvas>> {
        struct Placed {
            id: String,
            kind: String,
            source_hash: String,
            frame: Frame,
            z_index: i64,
            settings: String,
            lifecycle: String,
        }
        let rows: Vec<Placed> = self
            .tx
            .prepare(
                "SELECT id, kind, source_hash, x, y, width, height, z_index, settings, lifecycle \
                 FROM canvas_instances WHERE page_id = ?1 ORDER BY z_index, z_minor, created_at, id",
            )?
            .query_map([page], |r| {
                Ok(Placed {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    source_hash: r.get(2)?,
                    frame: Frame::new(r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?),
                    z_index: r.get(7)?,
                    settings: r.get(8)?,
                    lifecycle: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut present = Vec::new();
        for p in rows {
            let settings: Settings = match serde_json::from_str(&p.settings) {
                Ok(Value::Object(map)) => map,
                _ => continue,
            };
            if p.lifecycle == "archived" || is_true(&settings, HIDDEN) {
                continue;
            }
            let Some(kind) = CanvasKind::parse(&p.kind) else {
                continue;
            };
            present.push((p.id, kind, p.source_hash, p.frame, p.z_index, settings));
        }
        let authored: Vec<String> = self
            .page_settings(page)?
            .get("formTabOrder")
            .and_then(Value::as_array)
            .filter(|a| a.iter().all(Value::is_string))
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let mut order: Vec<usize> = Vec::new();
        for id in &authored {
            if let Some(i) = present.iter().position(|p| &p.0 == id) {
                if !order.contains(&i) {
                    order.push(i);
                }
            }
        }
        for i in 0..present.len() {
            if !order.contains(&i) {
                order.push(i);
            }
        }
        Ok(order
            .into_iter()
            .filter_map(|i| {
                let (id, kind, source_hash, frame, z_index, settings) = present[i].clone();
                let role = FormField::read(&settings)?;
                Some(FieldCanvas {
                    id,
                    kind,
                    source_hash,
                    frame,
                    z_index,
                    settings,
                    role,
                })
            })
            .collect())
    }

    fn answer_of(&self, field: &FieldCanvas, offset_minutes: i32) -> Result<Answer> {
        let label = field.role.label.as_deref().unwrap_or(&field.role.name);
        let content = |table: &str| -> Result<Option<String>> {
            Ok(self
                .tx
                .query_row(
                    &format!("SELECT data FROM {table} WHERE instance_id = ?1"),
                    [&field.id],
                    |r| r.get(0),
                )
                .optional()?)
        };
        match field.kind {
            CanvasKind::Database | CanvasKind::Spreadsheet => Err(Error::Invalid(format!(
                "\u{201c}{label}\u{201d} is a table; a table can't be a form field"
            ))),
            CanvasKind::RichText => {
                let doc = content("rich_text_instances")?
                    .and_then(|d| serde_json::from_str::<Value>(&d).ok())
                    .unwrap_or(Value::Null);
                Ok(Answer::Text(answer_text(&doc)))
            }
            CanvasKind::Calendar => {
                let day = field
                    .settings
                    .get("displayDayEpoch")
                    .and_then(Value::as_f64)
                    .filter(|d| d.is_finite())
                    .ok_or_else(|| {
                        Error::Invalid(format!(
                            "\u{201c}{label}\u{201d} isn't showing a date; set its displayDayEpoch"
                        ))
                    })?;
                Ok(Answer::Text(utc_date(
                    day as i64 + i64::from(offset_minutes) * 60,
                )))
            }
            CanvasKind::Sketch => {
                let doc = content("sketch_instances")?.unwrap_or_default();
                let strokes = serde_json::from_str::<Value>(&doc)
                    .ok()
                    .and_then(|v| v.get("strokes").and_then(Value::as_array).map(Vec::len))
                    .ok_or_else(|| {
                        Error::Invalid(format!(
                            "the content of \u{201c}{label}\u{201d} is damaged and couldn't be read"
                        ))
                    })?;
                Ok(if strokes == 0 {
                    Answer::Text(String::new())
                } else {
                    Answer::Sketch { doc }
                })
            }
            CanvasKind::Picture => Ok(
                if is_true(&field.settings, PLACEHOLDER) || field.source_hash.is_empty() {
                    Answer::Text(String::new())
                } else {
                    Answer::Picture {
                        hash: field.source_hash.clone(),
                    }
                },
            ),
        }
    }

    /// Submit the form on `page`, as DunneNote's Submit button does: read each field's current
    /// answer, check them, add any columns the answers table lacks (the form must own it), copy
    /// drawn and picture answers into hidden carrier canvases, and append one row with every
    /// answer as text and the `Submitted` time.
    pub fn submit_form(&mut self, page: &str, how: &Submission) -> Result<Submitted> {
        if self.node_kind(page)? != "page" {
            return Err(Error::Invalid(format!("{page} is not a page")));
        }
        let fields = self.field_canvases(page)?;
        if fields.is_empty() {
            return Err(Error::Invalid(
                "nothing to submit: no canvas on this page is a form field".into(),
            ));
        }
        let mut seen: Vec<String> = Vec::new();
        for f in &fields {
            if f.role.name.trim().eq_ignore_ascii_case(SUBMITTED_COLUMN) {
                return Err(Error::Invalid(format!(
                    "\u{201c}{}\u{201d} can't be a field name: every submission records its time in a \u{201c}{SUBMITTED_COLUMN}\u{201d} column",
                    f.role.name
                )));
            }
            let key = f.role.name.to_lowercase();
            if seen.contains(&key) {
                return Err(Error::Invalid(format!(
                    "two fields are both named \u{201c}{}\u{201d}",
                    f.role.name
                )));
            }
            seen.push(key);
        }
        let mut answers = Vec::with_capacity(fields.len());
        for f in &fields {
            answers.push(self.answer_of(f, how.utc_offset_minutes)?);
        }
        for (f, a) in fields.iter().zip(&answers) {
            if f.role.required && matches!(a, Answer::Text(t) if t.is_empty()) {
                let label = f.role.label.as_deref().unwrap_or(&f.role.name);
                return Err(Error::Invalid(format!(
                    "\u{201c}{label}\u{201d} is required"
                )));
            }
        }

        // The destination.
        let table = match read_form_destination(&self.page_settings(page)?) {
            Some(FormDestination::Canvas(id)) => id,
            Some(_) => {
                return Err(Error::Invalid(
                    "this form sends its answers to a file; submit it in DunneNote".into(),
                ))
            }
            None => {
                return Err(Error::Invalid(
                    "nothing to submit to: choose where this form's answers go".into(),
                ))
            }
        };
        let (kind, _) = self.canvas_kind(&table)?;
        if kind == CanvasKind::RichText {
            return Err(Error::Invalid(
                "this form appends its answers to a document; submit it in DunneNote".into(),
            ));
        }
        let owned = settings_keys::read_form_target(&self.canvas_settings(&table)?) == Some(true);

        // Columns, matched by name ignoring case (the first of equal names wins).
        let columns_of = |w: &Self| -> Result<Vec<(String, String)>> {
            let dataset: Option<String> =
                w.tx.query_row(
                    "SELECT id FROM datasets WHERE instance_id = ?1",
                    [&table],
                    |r| r.get(0),
                )
                .optional()?;
            let dataset =
                dataset.ok_or_else(|| Error::Invalid(format!("{table} is not a table")))?;
            Ok(w.tx
                .prepare(
                    "SELECT col_key, name FROM dataset_columns WHERE dataset_id = ?1 \
                     ORDER BY position, col_key",
                )?
                .query_map([&dataset], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?)
        };
        let find = |cols: &[(String, String)], name: &str| -> Option<String> {
            cols.iter()
                .find(|(_, n)| n.to_lowercase() == name.to_lowercase())
                .map(|(k, _)| k.clone())
        };
        let columns = columns_of(self)?;
        let mut wanted: Vec<(String, TypeHint)> = Vec::new();
        for f in &fields {
            if find(&columns, &f.role.name).is_none() {
                let hint = if f.kind == CanvasKind::Calendar {
                    TypeHint::Date
                } else {
                    TypeHint::Text
                };
                wanted.push((f.role.name.clone(), hint));
            }
        }
        if !wanted.is_empty() && !owned {
            return Err(Error::Invalid(format!(
                "the answers table has no \u{201c}{}\u{201d} column; add it, then submit",
                wanted[0].0
            )));
        }
        let rows: i64 = self.tx.query_row(
            "SELECT count(*) FROM dataset_rows r JOIN datasets d ON d.id = r.dataset_id \
             WHERE d.instance_id = ?1",
            [&table],
            |r| r.get(0),
        )?;
        if !wanted.is_empty() && rows > 0 && !how.confirm_new_columns {
            let names: Vec<&str> = wanted.iter().map(|(n, _)| n.as_str()).collect();
            return Err(Error::Invalid(format!(
                "submitting adds columns to an answers table that already has rows ({}); confirm to go ahead",
                names.join(", ")
            )));
        }
        if find(&columns, SUBMITTED_COLUMN).is_none() && owned {
            wanted.push((SUBMITTED_COLUMN.into(), TypeHint::Text));
        }
        let mut new_columns = Vec::new();
        for (name, hint) in &wanted {
            new_columns.push(self.add_column(&table, name, *hint, None)?);
        }
        let columns = columns_of(self)?;

        // The row: text answers in field order, then the stamp, then carriers in field order.
        let stamp = match &how.submitted {
            Some(s) => s.clone(),
            None => rfc3339_offset(now_secs(), how.utc_offset_minutes),
        };
        let mut cells = Map::new();
        let mut media = Vec::new();
        for (f, a) in fields.iter().zip(answers) {
            let key = find(&columns, &f.role.name).expect("columns were added above");
            match a {
                Answer::Text(t) => {
                    cells.insert(key, Value::String(t));
                }
                other => media.push((key, f, other)),
            }
        }
        if let Some(key) = find(&columns, SUBMITTED_COLUMN) {
            cells.insert(key, Value::String(stamp));
        }
        let mut hidden = Settings::new();
        hidden.insert(HIDDEN.into(), Value::Bool(true));
        let mut carriers = Vec::new();
        for (key, f, answer) in media {
            let (kind, hash) = match &answer {
                Answer::Picture { hash } => (CanvasKind::Picture, hash.clone()),
                _ => (CanvasKind::Sketch, self.put_blob(super::SKETCH_SENTINEL)?),
            };
            let carrier = self.insert_canvas_in_layer(
                page,
                kind,
                &hash,
                if f.frame.width > 0 && f.frame.height > 0 {
                    f.frame
                } else {
                    CARRIER_FALLBACK
                },
                &hidden,
                Some(f.z_index),
            )?;
            if let Answer::Sketch { doc } = &answer {
                // The drawing is copied byte for byte.
                self.tx.execute(
                    "INSERT INTO sketch_instances(instance_id, data, schema_version, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, unixepoch(), unixepoch())",
                    params![carrier, doc, super::CANVAS_SCHEMA_VERSION],
                )?;
            }
            let prefix = if kind == CanvasKind::Picture {
                "picture"
            } else {
                "sketch"
            };
            cells.insert(key, Value::String(format!("{prefix}:{carrier}")));
            carriers.push(carrier);
        }
        let row = self.insert_row(&table, &cells)?;
        Ok(Submitted {
            table,
            row,
            new_columns,
            carriers,
        })
    }

    /// Caption a picture as DunneNote's "Add caption" does: a transparent Rich Text canvas over
    /// the picture (at `placement`; the bottom edge for `User`), a layer above it, grouped with
    /// it, with the `caption` role. `doc` defaults to an empty document. Returns the caption.
    pub fn add_caption(
        &mut self,
        picture: &str,
        doc: Option<&Value>,
        placement: Placement,
    ) -> Result<String> {
        let page = self.expect_kind(picture, &[CanvasKind::Picture])?;
        let (x, y, width, height, z, group): (i64, i64, i64, i64, i64, Option<String>) =
            self.tx.query_row(
                "SELECT x, y, width, height, z_index, group_id FROM canvas_instances WHERE id = ?1",
                [picture],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )?;
        let anchor = Frame::new(x, y, width, height);
        let frame = caption_frame(anchor, CAPTION_HEIGHT, placement)
            .or_else(|| caption_frame(anchor, CAPTION_HEIGHT, Placement::Bottom))
            .expect("bottom always has a frame");
        let layer = self.top_layer(&page)?.max(z + 1);
        let data = match doc {
            Some(d) => {
                crate::payload::check_rich_text(d)?;
                d.to_string()
            }
            None => crate::payload::EMPTY_RICH_TEXT.to_string(),
        };
        let hash = self.put_blob(super::RICH_TEXT_SENTINEL)?;
        let mut settings = Settings::new();
        settings.insert(BACKGROUND_TRANSPARENT.into(), Value::Bool(true));
        let caption = self.insert_canvas_in_layer(
            &page,
            CanvasKind::RichText,
            &hash,
            frame,
            &settings,
            Some(layer),
        )?;
        self.tx.execute(
            "INSERT INTO rich_text_instances(instance_id, data, schema_version, created_at, updated_at) \
             VALUES (?1, ?2, ?3, unixepoch(), unixepoch())",
            params![caption, data, super::CANVAS_SCHEMA_VERSION],
        )?;
        match group {
            Some(g) => self.set_group(&[&caption], Some(&g))?,
            None => {
                self.add_group(&page, None, &[picture, &caption])?;
            }
        }
        let mut patch = Settings::new();
        patch.insert(CAPTION.into(), settings_keys::caption(picture, placement));
        self.merge_settings(&caption, &patch)?;
        Ok(caption)
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answer_text_matches_the_app() {
        let doc = json!({"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"  Ada "},{"type":"text","text":"Lovelace"}]},
            {"type":"paragraph","content":[
                {"type":"numFmt","attrs":{"raw":"12"}},
                {"type":"text","text":"x\u{A0}\u{A0}y"}]},
            {"type":"paragraph"}
        ]});
        assert_eq!(answer_text(&doc), "Ada Lovelace x y");
        assert_eq!(answer_text(&Value::Null), "");
    }

    #[test]
    fn caption_frames_match_the_app() {
        let a = Frame::new(100, 50, 200, 100);
        assert_eq!(
            caption_frame(a, 48, Placement::Bottom),
            Some(Frame::new(100, 102, 200, 48))
        );
        assert_eq!(
            caption_frame(a, 48, Placement::Top),
            Some(Frame::new(100, 50, 200, 48))
        );
        assert_eq!(
            caption_frame(a, 48, Placement::CornerTopRight),
            Some(Frame::new(202, 58, 90, 48))
        );
        assert_eq!(
            caption_frame(a, 48, Placement::CornerBottomLeft),
            Some(Frame::new(108, 94, 90, 48))
        );
        assert_eq!(
            caption_frame(a, 48, Placement::Movie),
            Some(Frame::new(130, 78, 140, 48))
        );
        assert_eq!(
            caption_frame(Frame::new(0, 0, 40, 20), 48, Placement::Bottom),
            Some(Frame::new(0, 0, 40, 20))
        );
        assert_eq!(caption_frame(a, 48, Placement::User), None);
    }
}
