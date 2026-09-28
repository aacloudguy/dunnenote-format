//! The reserved settings keys: roles a canvas plays (form field, answers table, caption, hidden
//! carrier, template placeholder) and the page settings forms use.
//!
//! Each builder writes a key's value the way DunneNote does, so a notebook written with this
//! library is byte-for-byte what the app would store. Readers should treat a key whose value does
//! not have the documented shape as absent, as DunneNote does. See `SPEC.md` section 7.

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::model::Settings;

pub const FORM_FIELD: &str = "formField";
pub const FORM_TARGET: &str = "formTarget";
pub const CAPTION: &str = "caption";
/// Only the literal `true` hides a canvas.
pub const HIDDEN: &str = "hidden";
/// Only the literal `true` marks a template's cleared picture or calendar.
pub const PLACEHOLDER: &str = "placeholder";
/// Only the literal `true` keeps a canvas's content when a page is made into a template.
pub const TEMPLATE_KEEP_CONTENT: &str = "templateKeepContent";
pub const BACKGROUND_TRANSPARENT: &str = "backgroundTransparent";

/// Longest form field name or label, in UTF-16 code units (as DunneNote counts).
pub const FIELD_NAME_MAX: usize = 128;

/// Whether `key` holds the literal `true`.
pub fn is_true(settings: &Settings, key: &str) -> bool {
    settings.get(key) == Some(&Value::Bool(true))
}

/// Trim, and cut to `max` UTF-16 code units (never splitting a character).
fn clean_text(value: &str, max: usize) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut units = 0;
    let mut out = String::new();
    for c in trimmed.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    Some(out)
}

// ---- form fields --------------------------------------------------------------------------------

/// Where a form field's label is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelDisplay {
    Off,
    Hover,
    Above,
    Below,
}

impl LabelDisplay {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "hover" => Some(Self::Hover),
            "above" => Some(Self::Above),
            "below" => Some(Self::Below),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Hover => "hover",
            Self::Above => "above",
            Self::Below => "below",
        }
    }
}

/// The `formField` role: this canvas is a field of its page's form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    /// The answers-table column the field fills (matched ignoring case).
    pub name: String,
    /// What the person filling the form sees; `None` shows `name`.
    pub label: Option<String>,
    pub required: bool,
    /// Overrides the page's `formLabelDisplay`.
    pub label_display: Option<LabelDisplay>,
}

impl FormField {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: None,
            required: false,
            label_display: None,
        }
    }

    /// Read a stored `formField` value; `None` when it is absent or has no usable name.
    pub fn read(settings: &Settings) -> Option<Self> {
        let rec = settings.get(FORM_FIELD)?.as_object()?;
        let name = clean_text(rec.get("name")?.as_str()?, FIELD_NAME_MAX)?;
        let label = rec
            .get("label")
            .and_then(Value::as_str)
            .and_then(|l| clean_text(l, FIELD_NAME_MAX))
            .filter(|l| *l != name);
        Some(Self {
            name,
            label,
            required: rec.get("required") == Some(&Value::Bool(true)),
            label_display: rec
                .get("labelDisplay")
                .and_then(Value::as_str)
                .and_then(LabelDisplay::parse),
        })
    }

    /// The stored value: `name`, then `label` (only when it differs), `required` (only when
    /// true) and `labelDisplay` (only when set). Fails when the name is blank.
    pub fn to_value(&self) -> Result<Value> {
        let name = clean_text(&self.name, FIELD_NAME_MAX)
            .ok_or_else(|| Error::Invalid("a form field needs a name".into()))?;
        let mut out = Map::new();
        out.insert("name".into(), json!(name));
        if let Some(label) = self
            .label
            .as_deref()
            .and_then(|l| clean_text(l, FIELD_NAME_MAX))
        {
            if label != name {
                out.insert("label".into(), json!(label));
            }
        }
        if self.required {
            out.insert("required".into(), json!(true));
        }
        if let Some(d) = self.label_display {
            out.insert("labelDisplay".into(), json!(d.as_str()));
        }
        Ok(Value::Object(out))
    }
}

/// The `formTarget` value marking a form's answers table. `owned_by_form` lets a submit add the
/// columns it needs.
pub fn form_target(owned_by_form: bool) -> Value {
    if owned_by_form {
        json!({"ownedByForm": true})
    } else {
        json!({})
    }
}

/// Whether a table is an answers table, and whether the form owns it.
pub fn read_form_target(settings: &Settings) -> Option<bool> {
    let rec = settings.get(FORM_TARGET)?.as_object()?;
    Some(rec.get("ownedByForm") == Some(&Value::Bool(true)))
}

// ---- captions -----------------------------------------------------------------------------------

/// Where a caption sits over its picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Bottom,
    Top,
    CornerTopLeft,
    CornerTopRight,
    CornerBottomLeft,
    CornerBottomRight,
    Movie,
    /// Placed by hand; not moved with the picture.
    User,
}

impl Placement {
    pub const ALL: [Placement; 8] = [
        Self::Bottom,
        Self::Top,
        Self::CornerTopLeft,
        Self::CornerTopRight,
        Self::CornerBottomLeft,
        Self::CornerBottomRight,
        Self::Movie,
        Self::User,
    ];
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::CornerTopLeft => "corner-tl",
            Self::CornerTopRight => "corner-tr",
            Self::CornerBottomLeft => "corner-bl",
            Self::CornerBottomRight => "corner-br",
            Self::Movie => "movie",
            Self::User => "user",
        }
    }
}

/// The `caption` value: this Rich Text canvas captions the picture `anchor`.
pub fn caption(anchor: &str, placement: Placement) -> Value {
    json!({"anchor": anchor, "placement": placement.as_str()})
}

/// Read a stored `caption` value: `(anchor, placement)`.
pub fn read_caption(settings: &Settings) -> Option<(String, Placement)> {
    let rec = settings.get(CAPTION)?.as_object()?;
    let anchor = rec.get("anchor")?.as_str().filter(|a| !a.is_empty())?;
    let placement = Placement::parse(rec.get("placement")?.as_str()?)?;
    Some((anchor.to_string(), placement))
}

// ---- page settings ------------------------------------------------------------------------------

/// Page settings keys, in the order DunneNote writes them (after any keys it does not know).
pub const PAGE_KEYS: [&str; 7] = [
    "hideCanvasFrames",
    "formTabOrder",
    "formFillMode",
    "formDestination",
    "formLabelDisplay",
    "hideSubmittedColumn",
    "dateDisplayFormat",
];

/// Read but never written: the form destination before `formDestination` replaced it.
pub const LEGACY_FORM_TARGET_ID: &str = "formTargetId";

pub const FILE_FORMATS: [&str; 3] = ["json", "markdown", "csv"];
pub const DATE_DISPLAY_FORMATS: [&str; 5] = ["iso", "dmy", "mdy", "numeric-dmy", "numeric-mdy"];

/// Where a form's answers go (`formDestination`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormDestination {
    /// A table canvas in the notebook.
    Canvas(String),
    /// A file DunneNote writes beside the notebook: `json`, `markdown` or `csv`.
    File(String),
    /// A file in a folder outside the notebook.
    External { format: String, path: String },
}

impl FormDestination {
    pub fn read(value: &Value) -> Option<Self> {
        let o = value.as_object()?;
        let text = |k: &str| o.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        let format = || text("format").filter(|f| FILE_FORMATS.contains(f));
        match o.get("kind")?.as_str()? {
            "canvas" => Some(Self::Canvas(text("id")?.into())),
            "file" => Some(Self::File(format()?.into())),
            "external" => Some(Self::External {
                format: format()?.into(),
                path: text("path")?.into(),
            }),
            _ => None,
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::Canvas(id) => json!({"kind": "canvas", "id": id}),
            Self::File(format) => json!({"kind": "file", "format": format}),
            Self::External { format, path } => {
                json!({"kind": "external", "format": format, "path": path})
            }
        }
    }
}

/// A page's form destination, falling back to the legacy `formTargetId`.
pub fn read_form_destination(page_settings: &Settings) -> Option<FormDestination> {
    page_settings
        .get("formDestination")
        .and_then(FormDestination::read)
        .or_else(|| {
            page_settings
                .get(LEGACY_FORM_TARGET_ID)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(|id| FormDestination::Canvas(id.into()))
        })
}

/// A known page key's value in the shape DunneNote keeps, or `None` when it is the default (or
/// unreadable, which DunneNote treats the same way).
fn page_value(key: &str, value: &Value) -> Option<Value> {
    match key {
        "hideCanvasFrames" | "formFillMode" | "hideSubmittedColumn" => {
            (value == &Value::Bool(true)).then(|| value.clone())
        }
        "formTabOrder" => {
            let items = value.as_array()?;
            (!items.is_empty() && items.iter().all(Value::is_string)).then(|| value.clone())
        }
        "formDestination" => FormDestination::read(value).map(|d| d.to_value()),
        "formLabelDisplay" => value
            .as_str()
            .and_then(LabelDisplay::parse)
            .map(|d| json!(d.as_str())),
        "dateDisplayFormat" => value
            .as_str()
            .filter(|f| DATE_DISPLAY_FORMATS.contains(f))
            .map(|f| json!(f)),
        _ => None,
    }
}

/// Check a patch's known page keys before merging: a value that is neither `null` (remove) nor
/// valid is refused rather than silently dropped.
pub(crate) fn check_page_patch(patch: &Settings) -> Result<()> {
    for (key, value) in patch {
        if key == LEGACY_FORM_TARGET_ID {
            return Err(Error::Invalid(
                "formTargetId is no longer written; set formDestination".into(),
            ));
        }
        if !PAGE_KEYS.contains(&key.as_str()) || value.is_null() {
            continue;
        }
        let valid = match key.as_str() {
            "hideCanvasFrames" | "formFillMode" | "hideSubmittedColumn" => value.is_boolean(),
            "formTabOrder" => value
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string)),
            _ => page_value(key, value).is_some(),
        };
        if !valid {
            return Err(Error::Invalid(format!(
                "page setting {key} cannot be {value}"
            )));
        }
    }
    Ok(())
}

/// Merge `patch` into stored page settings as DunneNote does: keys it does not know first, in
/// their stored order; then its own keys in [`PAGE_KEYS`] order, each only when not the default;
/// the legacy `formTargetId` folded into `formDestination`. `None` means the column is NULL.
pub(crate) fn merge_page(stored: Option<&Settings>, patch: &Settings) -> Option<Settings> {
    let empty = Settings::new();
    let stored = stored.unwrap_or(&empty);
    let mut known: Map<String, Value> = Map::new();
    for key in PAGE_KEYS {
        let value = match patch.get(key) {
            Some(Value::Null) => None,
            Some(v) => page_value(key, v),
            None if key == "formDestination" => read_form_destination(stored).map(|d| d.to_value()),
            None => stored.get(key).and_then(|v| page_value(key, v)),
        };
        if let Some(v) = value {
            known.insert(key.into(), v);
        }
    }
    let mut out = Settings::new();
    for (key, value) in stored.iter().chain(patch.iter()) {
        if PAGE_KEYS.contains(&key.as_str()) || key == LEGACY_FORM_TARGET_ID {
            continue;
        }
        match patch.get(key) {
            Some(Value::Null) => {}
            Some(v) => {
                out.insert(key.clone(), v.clone());
            }
            None => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    out.extend(known);
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Settings {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn form_field_is_written_like_the_app() {
        let mut f = FormField::new("  Surname ");
        f.label = Some("Surname".into());
        assert_eq!(f.to_value().unwrap().to_string(), r#"{"name":"Surname"}"#);
        f.label = Some("Your surname".into());
        f.required = true;
        f.label_display = Some(LabelDisplay::Above);
        assert_eq!(
            f.to_value().unwrap().to_string(),
            r#"{"name":"Surname","label":"Your surname","required":true,"labelDisplay":"above"}"#
        );
        assert!(FormField::new("  ").to_value().is_err());
        let long = "é".repeat(200);
        let v = FormField::new(long).to_value().unwrap();
        assert_eq!(v["name"].as_str().unwrap().chars().count(), FIELD_NAME_MAX);
    }

    #[test]
    fn page_settings_merge_orders_keys_and_drops_defaults() {
        let stored = obj(json!({
            "formFillMode": true, "zeta": 1, "formTargetId": "abc", "alpha": [1]
        }));
        let patch = obj(json!({"hideCanvasFrames": true, "alpha": null, "omega": "x"}));
        let out = merge_page(Some(&stored), &patch).unwrap();
        assert_eq!(
            Value::Object(out).to_string(),
            r#"{"zeta":1,"omega":"x","hideCanvasFrames":true,"formFillMode":true,"formDestination":{"kind":"canvas","id":"abc"}}"#
        );
        let cleared = merge_page(
            Some(&obj(json!({"formFillMode": true}))),
            &obj(json!({"formFillMode": false})),
        );
        assert_eq!(cleared, None);
    }

    #[test]
    fn page_patch_values_are_checked() {
        assert!(check_page_patch(&obj(json!({"formLabelDisplay": "sideways"}))).is_err());
        assert!(check_page_patch(&obj(json!({"formTargetId": "x"}))).is_err());
        assert!(check_page_patch(&obj(
            json!({"formDestination": {"kind": "file", "format": "md"}})
        ))
        .is_err());
        assert!(check_page_patch(&obj(json!({"formFillMode": false, "other": 3}))).is_ok());
    }
}
