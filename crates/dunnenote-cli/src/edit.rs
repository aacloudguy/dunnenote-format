//! Editing commands: each opens the notebook for writing, makes one change in one transaction and
//! prints the new item's id.

use dunnenote_format::settings_keys::{FormField, LabelDisplay, Placement};
use dunnenote_format::{
    ingest, payload, ArchiveReason, At, CanvasKind, Error, Frame, MetaValue, NodeKind, Notebook,
    Settings, Stroke, Submission, CALENDAR_SIZE, TABLE_SIZE,
};
use serde_json::{Map, Value};

use crate::Args;

fn bad_input(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

pub fn new_cmd(path: &str, name: Option<&str>) -> Result<(), Error> {
    let nb = Notebook::create(path, name)?;
    println!("{}", nb.notebook_node()?.id);
    eprintln!("created {path}");
    Ok(())
}

pub fn add_node(
    path: &str,
    parent: &str,
    name: &str,
    page: bool,
    args: &Args,
) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    let parent = if parent == "root" {
        nb.notebook_node()?.id
    } else {
        parent.to_string()
    };
    let at = if args.flag("--first") {
        At::Start
    } else {
        At::End
    };
    let with_text = page && !args.flag("--no-text");
    let id = nb.write(|w| {
        if !page {
            return w.add_section(&parent, name, at);
        }
        let id = w.add_page(&parent, name, at)?;
        if with_text {
            w.add_rich_text(&id, Frame::PAGE_TEXT, None, &Settings::new())?;
        }
        Ok(id)
    })?;
    println!("{id}");
    Ok(())
}

fn pair(args: &Args, flag: &str) -> Result<Option<(i64, i64)>, Error> {
    let Some(v) = args.value(flag) else {
        return Ok(None);
    };
    let parsed = v
        .split_once(',')
        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)));
    parsed
        .map(Some)
        .ok_or_else(|| bad_input(format!("{flag} takes two whole numbers, like {flag}=40,40")))
}

/// Below everything already on the page, at the left margin DunneNote uses.
fn next_free_spot(nb: &Notebook, page: &str) -> Result<(i64, i64), Error> {
    let bottom = nb
        .canvases(page)?
        .iter()
        .filter(|c| !c.is_hidden())
        .map(|c| c.y + c.height)
        .max();
    Ok(match bottom {
        None => (40, 40),
        Some(b) => (40, b + 20),
    })
}

fn read_input(file: &str) -> Result<String, Error> {
    if file == "-" {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
        Ok(s)
    } else {
        Ok(std::fs::read_to_string(file)?)
    }
}

fn rich_text_from_file(file: Option<&str>) -> Result<Option<Value>, Error> {
    let Some(file) = file else { return Ok(None) };
    let text = read_input(file)?;
    let lower = file.to_lowercase();
    Ok(Some(if lower.ends_with(".json") {
        serde_json::from_str(&text).map_err(|e| bad_input(format!("{file} is not JSON: {e}")))?
    } else if lower.ends_with(".txt") {
        payload::rich_text_from_plain(&text)
    } else {
        payload::rich_text_from_markdown(&text)
    }))
}

fn strokes_from_file(file: Option<&str>) -> Result<Vec<Stroke>, Error> {
    let Some(file) = file else {
        return Ok(Vec::new());
    };
    let v: Value = serde_json::from_str(&read_input(file)?)
        .map_err(|e| bad_input(format!("{file} is not JSON: {e}")))?;
    let list = v.get("strokes").cloned().unwrap_or(v);
    serde_json::from_value(list).map_err(|e| {
        bad_input(format!(
            "{file}: expected a list of strokes ({{\"id\", \"points\": [{{\"x\", \"y\", \"p\"}}], \"color\", \"width\", \"tool\"}}): {e}"
        ))
    })
}

pub fn add_canvas(
    path: &str,
    page: &str,
    kind: &str,
    file: Option<&str>,
    args: &Args,
) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    if nb.node(page)?.kind != NodeKind::Page {
        return Err(bad_input(format!("{page} is not a page")));
    }
    let (x, y) = match pair(args, "--at")? {
        Some(at) => at,
        None => next_free_spot(&nb, page)?,
    };
    let size = pair(args, "--size")?;
    let frame = |w: i64, h: i64| {
        let (w, h) = size.unwrap_or((w, h));
        Frame::new(x, y, w, h)
    };
    let id = match kind {
        "rich-text" | "text" => {
            let doc = rich_text_from_file(file)?;
            nb.write(|w| w.add_rich_text(page, frame(720, 320), doc.as_ref(), &Settings::new()))?
        }
        "sketch" => {
            let strokes = strokes_from_file(file)?;
            nb.write(|w| w.add_sketch(page, frame(560, 400), &strokes, &Settings::new()))?
        }
        "picture" => {
            let file = file.ok_or_else(|| bad_input("add-canvas picture needs an image file"))?;
            let image = std::fs::read(file)?;
            let mut settings = Settings::new();
            if let Some(alt) = args.value("--alt") {
                settings.insert("alt".into(), Value::String(alt.into()));
            }
            nb.write(|w| w.add_picture(page, (x, y), size, &image, &settings))?
        }
        "table" => {
            let (w, h) = TABLE_SIZE;
            let frame = frame(w, h);
            match file {
                None => nb.write(|w| w.add_table(page, frame, &Settings::new()))?,
                Some(file) => {
                    let bytes = std::fs::read(file)?;
                    let lower = file.to_lowercase();
                    if lower.ends_with(".csv") {
                        nb.write(|w| w.add_table_from_csv(page, frame, &bytes, &Settings::new()))?
                    } else if lower.ends_with(".json") || lower.ends_with(".jsonl") {
                        nb.write(|w| w.add_table_from_json(page, frame, &bytes, &Settings::new()))?
                    } else {
                        return Err(bad_input(format!(
                            "{file}: a table is imported from a .csv or .json file"
                        )));
                    }
                }
            }
        }
        "calendar" => {
            let file = file.ok_or_else(|| bad_input("add-canvas calendar needs an .ics file"))?;
            let ics = std::fs::read(file)?;
            let (w, h) = CALENDAR_SIZE;
            let frame = frame(w, h);
            let (id, skipped) =
                nb.write(|w| w.add_calendar(page, frame, &ics, &Settings::new()))?;
            if skipped > 0 {
                eprintln!("{skipped} events could not be read and were left out");
            }
            id
        }
        other => {
            return Err(bad_input(format!(
                "unknown canvas kind {other:?}; use rich-text, sketch, picture, table or calendar"
            )))
        }
    };
    println!("{id}");
    Ok(())
}

/// The key of the column named `col` (ignoring case), or `col` itself when it is a key.
fn column_key(nb: &Notebook, table: &str, col: &str) -> Result<String, Error> {
    let ds = nb
        .dataset(table)?
        .ok_or_else(|| bad_input(format!("{table} is not a table")))?;
    ds.columns
        .iter()
        .find(|c| c.col_key == col)
        .or_else(|| ds.columns.iter().find(|c| c.name.eq_ignore_ascii_case(col)))
        .map(|c| c.col_key.clone())
        .ok_or_else(|| bad_input(format!("table {table} has no column {col:?}")))
}

/// `dnfmt table <notebook> <table-id> add-row|set|add-column …`
pub fn table_cmd(path: &str, table: &str, op: &str, rest: &[&str]) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    let out = match (op, rest) {
        ("add-row", cells) => {
            let mut row = Map::new();
            for cell in cells {
                let (col, text) = cell.split_once('=').ok_or_else(|| {
                    bad_input(format!("{cell:?}: give cells as <column>=<value>"))
                })?;
                row.insert(column_key(&nb, table, col)?, ingest::cell_from_text(text));
            }
            nb.write(|w| w.insert_row(table, &row))?
        }
        ("set", [row, col, text]) => {
            let key = column_key(&nb, table, col)?;
            nb.write(|w| w.set_cell(table, row, &key, &ingest::cell_from_text(text)))?;
            row.to_string()
        }
        ("add-column", [name]) => {
            nb.write(|w| w.add_column(table, name, ingest::TypeHint::Unknown, None))?
        }
        _ => {
            return Err(bad_input(
                "use: table <notebook> <table-id> add-row [<column>=<value>…] | \
                 set <row-id> <column> <value> | add-column <name>",
            ))
        }
    };
    println!("{out}");
    Ok(())
}

fn root_or(nb: &Notebook, id: &str) -> Result<String, Error> {
    Ok(if id == "root" {
        nb.notebook_node()?.id
    } else {
        id.to_string()
    })
}

/// `dnfmt form <notebook> new|field|submit …`
pub fn form_cmd(path: &str, op: &str, rest: &[&str], args: &Args) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    match (op, rest) {
        ("new", [parent]) => {
            let parent = root_or(&nb, parent)?;
            let name = args.value("--name").unwrap_or("Untitled form");
            let (page, table) = nb.write(|w| w.add_form(&parent, name, At::End))?;
            println!("{page}");
            eprintln!("answers table {table}");
            Ok(())
        }
        ("field", [canvas]) => {
            if args.flag("--remove") {
                nb.write(|w| w.set_form_field(canvas, None))?;
                return Ok(());
            }
            let name = args
                .value("--name")
                .ok_or_else(|| bad_input("form field needs --name=<name> (or --remove)"))?;
            let mut field = FormField::new(name);
            field.label = args.value("--label").map(str::to_string);
            field.required = args.flag("--required");
            field.label_display =
                match args.value("--label-display") {
                    None => None,
                    Some(v) => Some(LabelDisplay::parse(v).ok_or_else(|| {
                        bad_input("--label-display is off, hover, above or below")
                    })?),
                };
            nb.write(|w| w.set_form_field(canvas, Some(&field)))?;
            Ok(())
        }
        ("submit", [page, answers @ ..]) => {
            // Each answer fills the field of that name (ignoring case) before submitting, as a
            // person would: text for a Rich Text field, @file for a Markdown/text file or, for a
            // Sketch field, a strokes JSON file.
            let mut fields: Vec<(String, String, CanvasKind)> = Vec::new();
            for c in nb.canvases(page)? {
                if let Some(role) = FormField::read(&c.settings) {
                    fields.push((role.name.to_lowercase(), c.id.clone(), c.kind));
                }
            }
            enum Fill {
                Text(String, serde_json::Value),
                Strokes(String, Vec<Stroke>),
            }
            let mut fills = Vec::new();
            for answer in answers {
                let (name, value) = answer.split_once('=').ok_or_else(|| {
                    bad_input(format!("{answer:?}: give answers as <field>=<value>"))
                })?;
                let (_, id, kind) = fields
                    .iter()
                    .find(|(n, _, _)| *n == name.to_lowercase())
                    .ok_or_else(|| bad_input(format!("this form has no field named {name:?}")))?;
                let file = value.strip_prefix('@');
                fills.push(match kind {
                    CanvasKind::RichText => {
                        let doc = match file {
                            Some(f) => rich_text_from_file(Some(f))?.expect("a file was given"),
                            None => payload::rich_text_from_plain(value),
                        };
                        Fill::Text(id.clone(), doc)
                    }
                    CanvasKind::Sketch => {
                        let f = file.ok_or_else(|| {
                            bad_input(format!("{name} is a sketch: answer it with @strokes.json"))
                        })?;
                        Fill::Strokes(id.clone(), strokes_from_file(Some(f))?)
                    }
                    other => {
                        return Err(bad_input(format!(
                            "{name} is a {} field; it is submitted as it is on the page",
                            other.display_name()
                        )))
                    }
                });
            }
            let how = Submission {
                submitted: args.value("--submitted").map(str::to_string),
                utc_offset_minutes: match args.value("--utc-offset") {
                    None => 0,
                    Some(v) => v.parse().map_err(|_| {
                        bad_input("--utc-offset is minutes east of UTC, like --utc-offset=-300")
                    })?,
                },
                confirm_new_columns: args.flag("--confirm"),
            };
            let done = nb.write(|w| {
                for fill in &fills {
                    match fill {
                        Fill::Text(id, doc) => w.set_rich_text(id, doc)?,
                        Fill::Strokes(id, strokes) => w.set_sketch(id, strokes)?,
                    }
                }
                w.submit_form(page, &how)
            })?;
            println!("{}", done.row);
            if !done.new_columns.is_empty() {
                eprintln!("added columns {}", done.new_columns.join(", "));
            }
            Ok(())
        }
        _ => Err(bad_input(
            "use: form <notebook> new <parent-id|root> [--name=<name>] | \
             field <canvas-id> --name=<name> [--label=…] [--required] | \
             submit <page-id> [<field>=<value>|<field>=@<file>…]",
        )),
    }
}

/// `dnfmt caption <notebook> <picture-id> [<text>|@<file>] [--placement=…]`
pub fn caption_cmd(
    path: &str,
    picture: &str,
    text: Option<&str>,
    args: &Args,
) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    let placement = match args.value("--placement") {
        None => Placement::Bottom,
        Some(p) => Placement::parse(p).ok_or_else(|| {
            bad_input("--placement is bottom, top, corner-tl, corner-tr, corner-bl, corner-br, movie or user")
        })?,
    };
    let doc = match text {
        None => None,
        Some(t) => match t.strip_prefix('@') {
            Some(file) => rich_text_from_file(Some(file))?,
            None => Some(payload::rich_text_from_plain(t)),
        },
    };
    let id = nb.write(|w| w.add_caption(picture, doc.as_ref(), placement))?;
    println!("{id}");
    Ok(())
}

/// `dnfmt template <notebook> make <page-id>` / `new <template-id> <parent-id|root>`
pub fn template_cmd(path: &str, op: &str, rest: &[&str]) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    let id = match (op, rest) {
        ("make", [page]) => nb.write(|w| w.make_template(page))?,
        ("new", [template, parent]) => {
            let parent = root_or(&nb, parent)?;
            nb.write(|w| w.new_from_template(template, &parent))?
        }
        _ => {
            return Err(bad_input(
                "use: template <notebook> make <page-id> | new <template-id> <parent-id|root>",
            ))
        }
    };
    println!("{id}");
    Ok(())
}

/// Is `id` a page, section or notebook (true) or a canvas (false)?
fn is_node(nb: &Notebook, id: &str) -> Result<bool, Error> {
    match nb.node(id) {
        Ok(_) => Ok(true),
        Err(Error::NotFound { .. }) => {
            nb.canvas(id)?;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// `dnfmt archive <notebook> <id> --reason=… [--note=…]` and `dnfmt retrieve <notebook> <id>`
pub fn archive_cmd(path: &str, id: &str, retrieve: bool, args: &Args) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    let node = is_node(&nb, id)?;
    if retrieve {
        return nb.write(|w| {
            if node {
                w.retrieve_node(id)
            } else {
                w.retrieve_canvas(id)
            }
        });
    }
    let reason = args
        .value("--reason")
        .and_then(ArchiveReason::parse)
        .ok_or_else(|| bad_input("archive needs --reason=superseded|wrong|irrelevant|other"))?;
    let note = args.value("--note");
    nb.write(|w| {
        if node {
            w.archive_node(id, reason, note)
        } else {
            w.archive_canvas(id, reason, note)
        }
    })
}

/// The tag source kind for an id: a node, a canvas, a table's dataset or a table row.
fn taggable(nb: &Notebook, id: &str) -> Result<&'static str, Error> {
    if is_node(nb, id).unwrap_or(false) {
        return Ok("node");
    }
    if nb.canvas(id).is_ok() {
        return Ok("instance");
    }
    let conn = nb.connection();
    let found = |sql: &str| -> bool { conn.query_row(sql, [id], |_| Ok(())).is_ok() };
    if found("SELECT 1 FROM dataset_rows WHERE id = ?1") {
        Ok("dataset_row")
    } else if found("SELECT 1 FROM datasets WHERE id = ?1") {
        Ok("dataset")
    } else {
        Err(bad_input(format!(
            "{id} is not a page, section, canvas, table or row"
        )))
    }
}

fn tag_id(nb: &Notebook, name: &str) -> Result<String, Error> {
    let folded = dunnenote_format::fold::fold(name.trim());
    nb.tags()?
        .into_iter()
        .find(|t| {
            t.name_folded == folded
                || t.aliases
                    .iter()
                    .any(|a| dunnenote_format::fold::fold(a) == folded)
        })
        .map(|t| t.id)
        .ok_or_else(|| bad_input(format!("there is no tag {name:?}")))
}

/// `dnfmt tag <notebook> add|rm <id> <name>` / `alias <name> <alias>` / `rename <name> <new>` /
/// `merge <from> <into>` / `delete <name>`
pub fn tag_cmd(path: &str, op: &str, rest: &[&str]) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    match (op, rest) {
        ("add", [id, name]) => {
            let kind = taggable(&nb, id)?;
            let tag = nb.write(|w| {
                let tag = w.tag(name)?;
                w.apply_tag(&tag, kind, id)?;
                Ok(tag)
            })?;
            println!("{tag}");
        }
        ("rm", [id, name]) => {
            let kind = taggable(&nb, id)?;
            let tag = tag_id(&nb, name)?;
            if !nb.write(|w| w.remove_tag(&tag, kind, id))? {
                return Err(bad_input(format!("{id} is not tagged {name:?}")));
            }
        }
        ("alias", [name, alias]) => {
            let tag = tag_id(&nb, name)?;
            nb.write(|w| w.add_tag_alias(&tag, alias))?;
        }
        ("rename", [name, new]) => {
            let tag = tag_id(&nb, name)?;
            nb.write(|w| w.rename_tag(&tag, new))?;
        }
        ("merge", [from, into]) => {
            let (loser, winner) = (tag_id(&nb, from)?, tag_id(&nb, into)?);
            nb.write(|w| w.merge_tags(&loser, &winner))?;
        }
        ("delete", [name]) => {
            let tag = tag_id(&nb, name)?;
            nb.write(|w| w.delete_tag(&tag))?;
        }
        _ => {
            return Err(bad_input(
                "use: tag <notebook> add|rm <id> <name> | alias <name> <alias> | \
                 rename <name> <new-name> | merge <from> <into> | delete <name>",
            ))
        }
    }
    Ok(())
}

/// `dnfmt meta <notebook> set <id> <key>=<value>` (text; `geo=<lat>,<lon>`;
/// `capture_time=<unix seconds>,<ISO 8601>`) / `rm <id> <key>`
pub fn meta_cmd(path: &str, op: &str, rest: &[&str]) -> Result<(), Error> {
    let mut nb = Notebook::open_writable(path)?;
    match (op, rest) {
        ("set", [id, pair]) => {
            let kind = taggable(&nb, id)?;
            let (key, raw) = pair
                .split_once('=')
                .ok_or_else(|| bad_input("give metadata as <key>=<value>"))?;
            let folded = dunnenote_format::fold::fold(key.trim());
            let value = match folded.as_str() {
                "geo" => {
                    let (lat, lon) = raw
                        .split_once(',')
                        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
                        .ok_or_else(|| bad_input("geo=<latitude>,<longitude>"))?;
                    MetaValue::Geo { lat, lon }
                }
                "capture_time" => {
                    let (secs, iso) = raw
                        .split_once(',')
                        .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().to_string())))
                        .ok_or_else(|| bad_input("capture_time=<unix seconds>,<ISO 8601>"))?;
                    MetaValue::Datetime {
                        epoch_secs: secs,
                        iso8601: iso,
                    }
                }
                _ => MetaValue::Text(raw.to_string()),
            };
            nb.write(|w| w.set_meta(kind, id, key, &value, "user"))
        }
        ("rm", [id, key]) => {
            let kind = taggable(&nb, id)?;
            if !nb.write(|w| w.remove_meta(kind, id, key))? {
                return Err(bad_input(format!("{id} has no {key:?}")));
            }
            Ok(())
        }
        _ => Err(bad_input(
            "use: meta <notebook> set <id> <key>=<value> | rm <id> <key>",
        )),
    }
}
