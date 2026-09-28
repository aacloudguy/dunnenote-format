//! Exporting a notebook to JSON, Markdown or CSV.
//!
//! - [`to_json`] is the whole notebook as one JSON document: every node, canvas, table row,
//!   event, tag and metadata entry, with settings exactly as stored. Blob bytes are referenced
//!   by hash, not embedded. The document shape is versioned ([`JSON_EXPORT_VERSION`]).
//! - [`to_markdown`] writes a folder you can open in any Markdown editor: one `.md` file per
//!   page in section folders, pictures copied out, sketches drawn as SVG.
//! - [`to_csv`] writes one CSV file per table (and per calendar).
//!
//! Markdown and CSV are readable copies, not backups: they skip archived content and hidden
//! canvases unless asked, and they drop layout. Use [`to_json`] (or copy the `.dunnenote`
//! folder) when nothing may be lost.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::model::{Canvas, CanvasKind, Dataset, Node, NodeKind, Stroke};
use crate::notebook::Notebook;
use crate::text::{markdown_with, rfc3339, utc_date, utc_datetime};

/// The `export_version` of the document [`to_json`] produces. Bumped on any incompatible change.
pub const JSON_EXPORT_VERSION: u32 = 1;

/// Which content the Markdown and CSV exports include.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Include archived pages and canvases (marked as archived).
    pub include_archived: bool,
}

/// What an export wrote.
#[derive(Debug, Default)]
pub struct Summary {
    /// Every file written, relative to the output folder.
    pub files: Vec<PathBuf>,
    pub pages: usize,
    pub tables: usize,
    pub calendars: usize,
    pub skipped_archived: usize,
}

// ---- JSON ---------------------------------------------------------------------------------------

/// The whole notebook as one JSON document.
pub fn to_json(nb: &Notebook) -> Result<Value> {
    let root = nb.notebook_node()?;
    let tags: Vec<Value> = nb
        .tags()?
        .into_iter()
        .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
        .collect();
    Ok(json!({
        "export_format": "dunnenote-export",
        "export_version": JSON_EXPORT_VERSION,
        "schema_version": nb.schema_version(),
        "manifest": nb.manifest(),
        "tags": tags,
        "root": node_json(nb, &root)?,
    }))
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn annotations(nb: &Notebook, kinds: &[&str], id: &str) -> Result<(Vec<String>, Vec<Value>)> {
    let mut tags = Vec::new();
    let mut meta = Vec::new();
    for kind in kinds {
        tags.extend(nb.tags_of(kind, id)?.into_iter().map(|t| t.name));
        meta.extend(nb.meta_of(kind, id)?.iter().map(to_value));
    }
    Ok((tags, meta))
}

fn node_json(nb: &Notebook, node: &Node) -> Result<Value> {
    let mut v = to_value(node);
    let (tags, meta) = annotations(nb, &["node"], &node.id)?;
    let obj = v
        .as_object_mut()
        .ok_or_else(|| Error::Malformed("node did not serialize".into()))?;
    obj.insert("tags".into(), json!(tags));
    obj.insert("meta".into(), json!(meta));
    if node.kind == NodeKind::Page {
        let canvases = nb
            .canvases(&node.id)?
            .iter()
            .map(|c| canvas_json(nb, c))
            .collect::<Result<Vec<_>>>()?;
        let groups: Vec<Value> = nb.groups(&node.id)?.iter().map(to_value).collect();
        obj.insert("canvases".into(), json!(canvases));
        obj.insert("groups".into(), json!(groups));
    } else {
        let children = nb
            .children(&node.id)?
            .iter()
            .map(|c| node_json(nb, c))
            .collect::<Result<Vec<_>>>()?;
        obj.insert("children".into(), json!(children));
    }
    Ok(v)
}

fn canvas_json(nb: &Notebook, c: &Canvas) -> Result<Value> {
    let content = match c.kind {
        CanvasKind::RichText => json!({"doc": nb.rich_text(&c.id)?.map(|r| r.doc)}),
        CanvasKind::Sketch => json!({"strokes": nb.sketch(&c.id)?.map(|s| s.data)}),
        CanvasKind::Picture => {
            let blob = nb.blob_info(&c.source_hash).ok();
            json!({
                "blob": blob.map(|b| json!({"hash": b.hash, "size_bytes": b.size_bytes})),
                "markup": nb.sketch(&c.id)?.map(|s| s.data),
            })
        }
        CanvasKind::Database | CanvasKind::Spreadsheet => {
            let ds = nb.dataset(&c.id)?;
            let mut v = ds.as_ref().map(to_value).unwrap_or(Value::Null);
            if let (Some(ds), Some(obj)) = (ds, v.as_object_mut()) {
                let mut row_tags = Map::new();
                for row in &ds.rows {
                    let names: Vec<String> = nb
                        .tags_of("dataset_row", &row.id)?
                        .into_iter()
                        .map(|t| t.name)
                        .collect();
                    if !names.is_empty() {
                        row_tags.insert(row.id.clone(), json!(names));
                    }
                }
                let (tags, _) = annotations(nb, &["dataset"], &ds.id)?;
                obj.insert("tags".into(), json!(tags));
                obj.insert("row_tags".into(), Value::Object(row_tags));
            }
            json!({"dataset": v})
        }
        CanvasKind::Calendar => {
            let events: Vec<Value> = nb.calendar_events(&c.id)?.iter().map(to_value).collect();
            json!({"source_ics": {"hash": c.source_hash}, "events": events})
        }
    };
    let mut v = to_value(c);
    let (tags, meta) = annotations(nb, &["canvas", "instance"], &c.id)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("tags".into(), json!(tags));
        obj.insert("meta".into(), json!(meta));
        obj.insert("content".into(), content);
    }
    Ok(v)
}

// ---- output folder helpers ----------------------------------------------------------------------

/// Create `out`, refusing a folder that already holds files: an export never overwrites.
fn prepare_dir(out: &Path) -> Result<()> {
    if out.exists() {
        if !out.is_dir() || fs::read_dir(out)?.next().is_some() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} exists and is not an empty folder", out.display()),
            )));
        }
    } else {
        fs::create_dir_all(out)?;
    }
    Ok(())
}

const RESERVED_WINDOWS: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file or folder name that is safe on macOS, Windows and Linux.
fn safe_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    s = s.trim().trim_end_matches('.').trim().to_string();
    if s.chars().count() > 100 {
        s = s
            .chars()
            .take(100)
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    if s.is_empty() || s.starts_with('.') {
        s.insert_str(0, "Untitled");
    }
    if RESERVED_WINDOWS.iter().any(|r| {
        s.split('.')
            .next()
            .is_some_and(|stem| stem.eq_ignore_ascii_case(r))
    }) {
        s.insert(0, '_');
    }
    s
}

/// Names already used in each folder, so two pages called "Notes" become `Notes.md` and
/// `Notes (2).md`. Compared case-insensitively, as macOS and Windows do.
#[derive(Default)]
struct Names(HashSet<(PathBuf, String)>);

impl Names {
    fn claim(&mut self, dir: &Path, stem: &str, ext: &str) -> String {
        let mut n = 1;
        loop {
            let candidate = if n == 1 {
                format!("{stem}{ext}")
            } else {
                format!("{stem} ({n}){ext}")
            };
            if self.0.insert((dir.to_path_buf(), candidate.to_lowercase())) {
                return candidate;
            }
            n += 1;
        }
    }
}

/// `../` repeated once per folder level, to reach the export root from a page file.
fn up(depth: usize) -> String {
    "../".repeat(depth)
}

fn picture_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.starts_with(b"GIF8") {
        "gif"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        match &bytes[8..12] {
            b"heic" | b"heix" | b"mif1" => "heic",
            b"avif" => "avif",
            _ => "bin",
        }
    } else {
        "bin"
    }
}

/// A colour safe to put in an SVG attribute: `#rgb`/`#rrggbb`/`#rrggbbaa` or a plain name.
fn svg_colour(c: &str) -> &str {
    let hex = c.len() > 1
        && c.len() <= 9
        && c.starts_with('#')
        && c[1..].bytes().all(|b| b.is_ascii_hexdigit());
    let name = !c.is_empty() && c.len() <= 20 && c.bytes().all(|b| b.is_ascii_alphabetic());
    if hex || name {
        c
    } else {
        "#000000"
    }
}

/// A sketch drawn as SVG at the canvas's size. Stroke points are fractions of the canvas.
pub fn sketch_svg(strokes: &[Stroke], width: i64, height: i64) -> String {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {w} {h}\">\n"
    );
    for s in strokes {
        let mut d = String::new();
        for (i, p) in s.points.iter().enumerate() {
            let cmd = if i == 0 { 'M' } else { 'L' };
            d.push_str(&format!("{cmd}{:.2} {:.2} ", p.x * w, p.y * h));
        }
        let width = if s.width.is_finite() && s.width > 0.0 {
            s.width
        } else {
            1.0
        };
        out.push_str(&format!(
            "  <path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\" \
             stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\n",
            d.trim_end(),
            svg_colour(&s.color)
        ));
    }
    out.push_str("</svg>\n");
    out
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_line(fields: &[String]) -> String {
    let mut line = fields
        .iter()
        .map(|f| csv_field(f))
        .collect::<Vec<_>>()
        .join(",");
    line.push_str("\r\n");
    line
}

fn dataset_csv(ds: &Dataset) -> String {
    let mut out = csv_line(
        &ds.columns
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>(),
    );
    for row in &ds.rows {
        out.push_str(&csv_line(&ds.row_strings(row)));
    }
    out
}

fn md_cell(s: &str) -> String {
    crate::text::escape_md(s).replace(['\r', '\n'], " ")
}

fn dataset_md(ds: &Dataset) -> String {
    if ds.columns.is_empty() {
        return "*(empty table)*".into();
    }
    let header: Vec<String> = ds.columns.iter().map(|c| md_cell(&c.name)).collect();
    let mut out = format!(
        "| {} |\n|{}\n",
        header.join(" | "),
        " --- |".repeat(header.len())
    );
    for row in &ds.rows {
        let cells: Vec<String> = ds.row_strings(row).iter().map(|c| md_cell(c)).collect();
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    out.truncate(out.trim_end().len());
    out
}

/// Pages to export with the folder path (section names) leading to each.
fn pages_with_paths(
    nb: &Notebook,
    opts: Options,
    summary: &mut Summary,
) -> Result<Vec<(Vec<String>, Node)>> {
    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut archived_depth: Option<usize> = None;
    for (depth, node) in nb.walk()? {
        if depth == 0 {
            continue;
        }
        path.truncate(depth - 1);
        if archived_depth.is_some_and(|d| depth <= d) {
            archived_depth = None;
        }
        let archived = node.is_archived || archived_depth.is_some();
        if archived && !opts.include_archived {
            if node.kind == NodeKind::Page {
                summary.skipped_archived += 1;
            } else if archived_depth.is_none() {
                archived_depth = Some(depth);
            }
            continue;
        }
        match node.kind {
            NodeKind::Page => out.push((path.clone(), node)),
            _ => path.push(node.name.clone()),
        }
    }
    Ok(out)
}

fn visible(c: &Canvas, opts: Options, summary: &mut Summary) -> bool {
    if c.is_hidden() {
        return false;
    }
    if c.is_archived() && !opts.include_archived {
        summary.skipped_archived += 1;
        return false;
    }
    true
}

/// Reading order on a page: top to bottom, then left to right.
fn reading_order(mut canvases: Vec<Canvas>) -> Vec<Canvas> {
    canvases.sort_by_key(|c| (c.y, c.x, c.z_index, c.z_minor));
    canvases
}

fn yaml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

// ---- Markdown -----------------------------------------------------------------------------------

/// Write the notebook as a folder of Markdown files. `out` must not exist or must be empty.
pub fn to_markdown(nb: &Notebook, out: &Path, opts: Options) -> Result<Summary> {
    prepare_dir(out)?;
    let mut summary = Summary::default();
    let mut names = Names::default();
    let assets = out.join("assets");
    let root = nb.notebook_node()?;
    let mut index = format!("# {}\n\n", md_cell(&root.name));
    let mut last_path: Vec<String> = Vec::new();

    for (path, page) in pages_with_paths(nb, opts, &mut summary)? {
        // Folders for the sections, claimed once each.
        let mut dir = out.to_path_buf();
        let mut rel = PathBuf::new();
        let common = path
            .iter()
            .zip(&last_path)
            .take_while(|(a, b)| a == b)
            .count();
        for (i, section) in path.iter().enumerate() {
            if i >= common {
                index.push_str(&format!("{}- **{}**\n", "  ".repeat(i), md_cell(section)));
            }
            let folder = safe_name(section);
            dir.push(&folder);
            rel.push(&folder);
        }
        last_path.clone_from(&path);
        fs::create_dir_all(&dir)?;
        let file = names.claim(&rel, &safe_name(&page.name), ".md");
        let rel_file = rel.join(&file);
        let depth = path.len();

        let mut md = String::from("---\n");
        md.push_str(&format!("title: {}\n", yaml_str(&page.name)));
        md.push_str(&format!("dunnenote_id: {}\n", yaml_str(&page.id)));
        let tags: Vec<String> = nb
            .tags_of("node", &page.id)?
            .into_iter()
            .map(|t| t.name)
            .collect();
        if !tags.is_empty() {
            let list: Vec<String> = tags.iter().map(|t| yaml_str(t)).collect();
            md.push_str(&format!("tags: [{}]\n", list.join(", ")));
        }
        if page.is_template {
            md.push_str("template: true\n");
        }
        if page.is_archived {
            md.push_str("archived: true\n");
        }
        md.push_str("---\n\n");
        md.push_str(&format!("# {}\n", md_cell(&page.name)));

        for c in reading_order(nb.canvases(&page.id)?) {
            if !visible(&c, opts, &mut summary) {
                continue;
            }
            let block = canvas_md(nb, &c, &assets, depth, &mut summary)?;
            if !block.is_empty() {
                md.push('\n');
                if c.is_archived() {
                    md.push_str("> *Archived:*\n\n");
                }
                md.push_str(&block);
                md.push('\n');
            }
        }
        fs::write(dir.join(&file), md)?;
        index.push_str(&format!(
            "{}- [{}](<{}>)\n",
            "  ".repeat(depth),
            md_cell(&page.name),
            rel_file.to_string_lossy().replace('\\', "/")
        ));
        summary.files.push(rel_file);
        summary.pages += 1;
    }
    fs::write(out.join("index.md"), index)?;
    summary.files.push(PathBuf::from("index.md"));
    Ok(summary)
}

fn canvas_md(
    nb: &Notebook,
    c: &Canvas,
    assets: &Path,
    depth: usize,
    summary: &mut Summary,
) -> Result<String> {
    Ok(match c.kind {
        CanvasKind::RichText => nb
            .rich_text(&c.id)?
            .map(|r| markdown_with(&r.doc, 1))
            .unwrap_or_default(),
        CanvasKind::Sketch => {
            let strokes = nb.sketch(&c.id)?.map(|s| s.strokes()).unwrap_or_default();
            fs::create_dir_all(assets)?;
            let file = format!("sketch-{}.svg", c.id);
            fs::write(assets.join(&file), sketch_svg(&strokes, c.width, c.height))?;
            summary.files.push(Path::new("assets").join(&file));
            format!("![Sketch](<{}assets/{file}>)", up(depth))
        }
        CanvasKind::Picture => match nb.read_blob(&c.source_hash) {
            Ok(bytes) => {
                let ext = picture_ext(&bytes);
                fs::create_dir_all(assets)?;
                let file = format!("{}.{ext}", c.source_hash);
                let path = assets.join(&file);
                if !path.exists() {
                    fs::write(&path, &bytes)?;
                    summary.files.push(Path::new("assets").join(&file));
                }
                let alt = c
                    .settings
                    .get("alt")
                    .and_then(Value::as_str)
                    .unwrap_or("Picture");
                format!("![{}](<{}assets/{file}>)", md_cell(alt), up(depth))
            }
            Err(_) => "*(picture missing from the notebook)*".into(),
        },
        CanvasKind::Database | CanvasKind::Spreadsheet => match nb.dataset(&c.id)? {
            Some(ds) => {
                summary.tables += 1;
                dataset_md(&ds)
            }
            None => String::new(),
        },
        CanvasKind::Calendar => {
            let mut lines = Vec::new();
            for ev in nb.calendar_events(&c.id)? {
                let when = if ev.all_day {
                    let last = (ev.end_utc - 1).max(ev.start_utc);
                    if utc_date(last) == utc_date(ev.start_utc) {
                        utc_date(ev.start_utc)
                    } else {
                        format!("{} – {}", utc_date(ev.start_utc), utc_date(last))
                    }
                } else {
                    format!(
                        "{} – {}",
                        utc_datetime(ev.start_utc),
                        utc_datetime(ev.end_utc)
                    )
                };
                let mut line = format!("- **{}** — {when}", md_cell(&ev.summary));
                if !ev.location.is_empty() {
                    line.push_str(&format!(" · {}", md_cell(&ev.location)));
                }
                if let Some(rule) = ev.rrule_text.as_deref().filter(|r| !r.is_empty()) {
                    line.push_str(&format!(" · repeats ({})", md_cell(rule)));
                }
                if !ev.description.is_empty() {
                    line.push_str(&format!("\n  {}", md_cell(&ev.description)));
                }
                lines.push(line);
            }
            if lines.is_empty() {
                "*(no events)*".into()
            } else {
                lines.join("\n")
            }
        }
    })
}

// ---- CSV ----------------------------------------------------------------------------------------

/// Write one CSV file per table, and one per calendar. `out` must not exist or must be empty.
///
/// Files are named `<section> - <page> - <table>.csv` (the table part is `Table`, `Table 2`, …
/// or `Calendar`). Cells are written as text, one header row of column names first.
pub fn to_csv(nb: &Notebook, out: &Path, opts: Options) -> Result<Summary> {
    prepare_dir(out)?;
    let mut summary = Summary::default();
    let mut names = Names::default();
    for (path, page) in pages_with_paths(nb, opts, &mut summary)? {
        let mut prefix: Vec<String> = path.clone();
        prefix.push(page.name.clone());
        let stem = safe_name(&prefix.join(" - "));
        let mut tables = 0;
        let mut calendars = 0;
        for c in reading_order(nb.canvases(&page.id)?) {
            if !visible(&c, opts, &mut summary) {
                continue;
            }
            let (label, body) = match c.kind {
                CanvasKind::Database | CanvasKind::Spreadsheet => {
                    let Some(ds) = nb.dataset(&c.id)? else {
                        continue;
                    };
                    tables += 1;
                    let label = if tables == 1 {
                        "Table".to_string()
                    } else {
                        format!("Table {tables}")
                    };
                    (label, dataset_csv(&ds))
                }
                CanvasKind::Calendar => {
                    calendars += 1;
                    summary.calendars += 1;
                    let label = if calendars == 1 {
                        "Calendar".to_string()
                    } else {
                        format!("Calendar {calendars}")
                    };
                    let mut body = csv_line(
                        &[
                            "Summary",
                            "Start",
                            "End",
                            "All day",
                            "Location",
                            "Description",
                            "Repeats",
                            "UID",
                        ]
                        .map(String::from),
                    );
                    for ev in nb.calendar_events(&c.id)? {
                        body.push_str(&csv_line(&[
                            ev.summary.clone(),
                            if ev.all_day {
                                utc_date(ev.start_utc)
                            } else {
                                rfc3339(ev.start_utc)
                            },
                            if ev.all_day {
                                utc_date(ev.end_utc)
                            } else {
                                rfc3339(ev.end_utc)
                            },
                            ev.all_day.to_string(),
                            ev.location.clone(),
                            ev.description.clone(),
                            ev.rrule_text.clone().unwrap_or_default(),
                            ev.uid.clone().unwrap_or_default(),
                        ]));
                    }
                    (label, body)
                }
                _ => continue,
            };
            let file = names.claim(Path::new(""), &format!("{stem} - {label}"), ".csv");
            fs::write(out.join(&file), body)?;
            summary.files.push(PathBuf::from(file));
        }
        summary.tables += tables;
        summary.pages += 1;
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StrokePoint;

    #[test]
    fn names_are_safe_and_unique() {
        assert_eq!(safe_name("a/b:c*?"), "a-b-c--");
        assert_eq!(safe_name("  . "), "Untitled");
        assert_eq!(safe_name(".hidden"), "Untitled.hidden");
        assert_eq!(safe_name("con"), "_con");
        assert_eq!(safe_name("Notes."), "Notes");
        let mut n = Names::default();
        assert_eq!(n.claim(Path::new("s"), "Notes", ".md"), "Notes.md");
        assert_eq!(n.claim(Path::new("s"), "notes", ".md"), "notes (2).md");
        assert_eq!(n.claim(Path::new("t"), "Notes", ".md"), "Notes.md");
    }

    #[test]
    fn csv_quotes_only_when_needed() {
        assert_eq!(
            csv_line(&["a".into(), "b,c".into(), "say \"hi\"".into(), "x\ny".into()]),
            "a,\"b,c\",\"say \"\"hi\"\"\",\"x\ny\"\r\n"
        );
    }

    #[test]
    fn svg_scales_points_and_refuses_odd_colours() {
        let s = Stroke {
            id: "s".into(),
            points: vec![
                StrokePoint {
                    x: 0.5,
                    y: 0.25,
                    p: 0.5,
                },
                StrokePoint {
                    x: 1.0,
                    y: 1.0,
                    p: 0.5,
                },
            ],
            color: "\"/><script>".into(),
            width: 2.0,
            tool: "pen".into(),
        };
        let svg = sketch_svg(&[s], 200, 100);
        assert!(svg.contains("d=\"M100.00 25.00 L200.00 100.00\""));
        assert!(svg.contains("stroke=\"#000000\""));
        assert!(!svg.contains("script"));
    }
}
