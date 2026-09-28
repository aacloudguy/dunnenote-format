//! Editing commands: each opens the notebook for writing, makes one change in one transaction and
//! prints the new item's id.

use dunnenote_format::{
    ingest, payload, At, Error, Frame, NodeKind, Notebook, Settings, Stroke, TABLE_SIZE,
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
        other => {
            return Err(bad_input(format!(
                "unknown canvas kind {other:?}; use rich-text, sketch, picture or table"
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
