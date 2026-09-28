//! Plain data types for what is stored in a notebook.
//!
//! Every timestamp is Unix seconds (UTC). Every id is a 36-character UUID string. `settings`
//! objects are kept as JSON maps in their stored key order, including keys this library does not
//! know: writers built on it must merge into them, never replace them with a subset.

use serde::Serialize;
use serde_json::{Map, Value};

pub type Settings = Map<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Notebook,
    Group,
    Page,
}

impl NodeKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "notebook" => Some(Self::Notebook),
            "group" => Some(Self::Group),
            "page" => Some(Self::Page),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notebook => "notebook",
            Self::Group => "group",
            Self::Page => "page",
        }
    }
}

/// A notebook, section group or page (the `nodes` table).
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub parent_id: Option<String>,
    pub name: String,
    /// Sibling order key. Siblings sort by plain byte order of this string.
    pub position: String,
    pub child_count: i64,
    pub is_template: bool,
    pub is_archived: bool,
    pub archive_reason: Option<String>,
    pub archive_note: Option<String>,
    pub archived_at: Option<i64>,
    /// Page settings (forms, display options); `None` when never set.
    pub settings: Option<Settings>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CanvasKind {
    RichText,
    Sketch,
    Picture,
    /// Shown in DunneNote as a read-only "Data Table".
    Database,
    /// Shown in DunneNote as an "Editable table".
    Spreadsheet,
    Calendar,
}

impl CanvasKind {
    pub const ALL: [CanvasKind; 6] = [
        Self::RichText,
        Self::Sketch,
        Self::Picture,
        Self::Database,
        Self::Spreadsheet,
        Self::Calendar,
    ];

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RichText => "rich_text",
            Self::Sketch => "sketch",
            Self::Picture => "picture",
            Self::Database => "database",
            Self::Spreadsheet => "spreadsheet",
            Self::Calendar => "calendar",
        }
    }
    /// The name DunneNote shows for this kind.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::RichText => "Rich Text",
            Self::Sketch => "Sketch",
            Self::Picture => "Picture",
            Self::Database => "Data Table",
            Self::Spreadsheet => "Editable table",
            Self::Calendar => "Calendar",
        }
    }
}

/// One canvas on a page (the `canvas_instances` table).
#[derive(Debug, Clone, Serialize)]
pub struct Canvas {
    pub id: String,
    pub page_id: String,
    pub kind: CanvasKind,
    /// SHA-256 of the canvas's source blob: the picture, the imported file, the `.ics`, or a
    /// fixed sentinel for kinds whose content lives in the database.
    pub source_hash: String,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    /// Layer: canvases stack by `(z_index, z_minor)`, then creation order.
    pub z_index: i64,
    pub z_minor: i64,
    pub settings: Settings,
    pub schema_version: i64,
    pub group_id: Option<String>,
    /// `"active"` or `"archived"`.
    pub lifecycle: String,
    pub lifecycle_reason: Option<String>,
    pub lifecycle_note: Option<String>,
    pub lifecycle_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Canvas {
    pub fn is_archived(&self) -> bool {
        self.lifecycle == "archived"
    }
    /// Hidden canvases (e.g. a form's answer carriers) are real content that DunneNote does not
    /// draw on the page. They are not orphans.
    pub fn is_hidden(&self) -> bool {
        self.settings.get("hidden") == Some(&Value::Bool(true))
    }
}

/// Rich text: a ProseMirror document (`{"type":"doc","content":[…]}`).
#[derive(Debug, Clone, Serialize)]
pub struct RichText {
    pub doc: Value,
    pub schema_version: i64,
    pub updated_at: i64,
}

/// Sketch strokes (also a picture's markup layer). `data` is the stored JSON; [`Sketch::strokes`]
/// parses the v1 shape.
#[derive(Debug, Clone, Serialize)]
pub struct Sketch {
    pub data: Value,
    pub schema_version: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct Stroke {
    pub id: String,
    /// Points with `x`, `y` normalised to the canvas (0..1) and pen pressure `p` in (0, 1].
    pub points: Vec<StrokePoint>,
    pub color: String,
    pub width: f64,
    pub tool: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, serde::Deserialize)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    pub p: f64,
}

impl Sketch {
    /// Strokes of a v1 sketch. Strokes that do not parse are skipped, never an error: DunneNote
    /// itself upgrades older shapes at read time.
    pub fn strokes(&self) -> Vec<Stroke> {
        self.data
            .get("strokes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| serde_json::from_value(s.clone()).ok())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A table: the rows behind a Data Table or an Editable table.
#[derive(Debug, Clone, Serialize)]
pub struct Dataset {
    pub id: String,
    pub instance_id: String,
    /// `"table"` or `"list"`.
    pub shape: String,
    /// Where it came from, e.g. `"csv"`, `"json"`, `"paste"`.
    pub source_kind: String,
    pub row_count: i64,
    pub schema_version: i64,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub id: String,
    /// Stable key used in each row's `cells` object.
    pub col_key: String,
    pub name: String,
    /// `text`, `number`, `date`, `boolean` or `unknown`.
    pub type_hint: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub id: String,
    pub seq: i64,
    /// Cell values keyed by [`Column::col_key`].
    pub cells: Map<String, Value>,
}

impl Dataset {
    /// Cells of `row` in column order, as display strings (missing cells are empty).
    pub fn row_strings(&self, row: &Row) -> Vec<String> {
        self.columns
            .iter()
            .map(|c| match row.cells.get(&c.col_key) {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CalendarEvent {
    pub id: String,
    pub instance_id: String,
    pub uid: Option<String>,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub all_day: bool,
    pub tzid: Option<String>,
    /// 0-based index of the `VEVENT` in the source `.ics`.
    pub source_ordinal: Option<i64>,
    pub url: Option<String>,
    pub organizer_value: Option<String>,
    pub organizer_cn: Option<String>,
    pub status: Option<String>,
    pub categories: Option<String>,
    pub rrule_text: Option<String>,
    pub attendees: Vec<Attendee>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attendee {
    pub ordinal: i64,
    pub value: String,
    pub cn: Option<String>,
    pub role: Option<String>,
    pub partstat: Option<String>,
    pub rsvp: bool,
}

/// Canvases that move and select as one. Groups nest.
#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub id: String,
    pub page_id: String,
    pub parent_group_id: Option<String>,
    pub settings: Settings,
    pub schema_version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub aliases: Vec<String>,
}

/// Typed metadata attached to an item (`item_meta`).
#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub key: String,
    pub value_text: Option<String>,
    pub value_num: Option<f64>,
    pub value_num2: Option<f64>,
    /// Who set it, e.g. `"user"` or `"exif"`.
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlobInfo {
    pub hash: String,
    pub size_bytes: i64,
    pub refcount: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub sections: i64,
    pub pages: i64,
    pub canvases: i64,
    pub canvases_by_kind: Vec<(String, i64)>,
    pub archived_pages: i64,
    pub archived_canvases: i64,
    pub templates: i64,
    pub tags: i64,
    pub blobs: i64,
    pub blob_bytes: i64,
}
