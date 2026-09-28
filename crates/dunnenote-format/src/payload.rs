//! Canvas payloads a writer produces: rich text documents and sketch strokes.
//!
//! **Rich text** is a ProseMirror document in DunneNote's editor schema (append-only; nodes and
//! marks are never renamed or removed). [`check_rich_text`] accepts exactly the documents that
//! DunneNote loads without loss: a document it would refuse (an unknown node or mark, an empty
//! text node, a missing required attribute) is rejected, and so is one it would load but alter
//! (an undeclared attribute is silently dropped by the editor; a malformed content structure is
//! repaired on the next edit). Attribute values are held to the forms DunneNote itself writes.
//!
//! **Sketches** (and a picture's markup layer) are `{"v":1,"strokes":[…]}`. [`sketch_json`]
//! serializes strokes byte-for-byte as DunneNote does: fixed key order, coordinates clamped to
//! 0..1 and rounded to 4 places, pressure to 2 places with a floor of 0.01, `tool` always `"pen"`.

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::model::Stroke;

/// Largest stored rich text document or sketch, in bytes (DunneNote refuses larger ones).
pub const MAX_DOC_BYTES: usize = 8 * 1024 * 1024;

/// A new, empty rich text canvas: one empty paragraph. Exactly what DunneNote seeds.
pub const EMPTY_RICH_TEXT: &str = r#"{"type":"doc","content":[{"type":"paragraph"}]}"#;

/// A new, empty sketch. Exactly what DunneNote seeds.
pub const EMPTY_SKETCH: &str = r#"{"v":1,"strokes":[]}"#;

/// Paragraph and heading alignments.
pub const ALIGNS: [&str; 4] = ["left", "center", "right", "justify"];

/// Font keys for the `font_family` mark.
pub const FONT_KEYS: [&str; 9] = [
    "inter",
    "source-sans-3",
    "public-sans",
    "literata",
    "source-serif-4",
    "lora",
    "jetbrains-mono",
    "ibm-plex-mono",
    "fira-code",
];

/// Marks in schema rank order; DunneNote sorts a text node's marks into this order.
pub const MARKS: [&str; 9] = [
    "strong",
    "em",
    "underline",
    "strike",
    "link",
    "font_family",
    "font_size",
    "text_color",
    "highlight",
];

// ---- rich text ------------------------------------------------------------------------------

/// Check that `doc` is a rich text document DunneNote loads without loss. The error names the
/// first problem and where it is (a path such as `doc.content[2].content[0]`).
pub fn check_rich_text(doc: &Value) -> Result<()> {
    let obj = node_object(doc, "doc")?;
    expect_type(obj, "doc", "doc")?;
    only_keys(obj, &["type", "content"], "doc")?;
    let blocks = children(obj, "doc")?;
    if blocks.is_empty() {
        return Err(bad("doc", "a document needs at least one block"));
    }
    for (i, b) in blocks.iter().enumerate() {
        check_block(b, &format!("doc.content[{i}]"))?;
    }
    let size = serde_json::to_string(doc).map(|s| s.len()).unwrap_or(0);
    if size > MAX_DOC_BYTES {
        return Err(Error::Invalid(format!(
            "the document is {size} bytes; DunneNote stores at most {MAX_DOC_BYTES}"
        )));
    }
    Ok(())
}

fn bad(at: &str, what: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("rich text {at}: {what}"))
}

fn node_object<'a>(v: &'a Value, at: &str) -> Result<&'a Map<String, Value>> {
    v.as_object().ok_or_else(|| bad(at, "is not a JSON object"))
}

fn node_type<'a>(obj: &'a Map<String, Value>, at: &str) -> Result<&'a str> {
    obj.get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| bad(at, "has no \"type\""))
}

fn expect_type(obj: &Map<String, Value>, want: &str, at: &str) -> Result<()> {
    let t = node_type(obj, at)?;
    if t != want {
        return Err(bad(at, format!("is a {t:?}, expected a {want:?}")));
    }
    Ok(())
}

fn only_keys(obj: &Map<String, Value>, allowed: &[&str], at: &str) -> Result<()> {
    match obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(bad(at, format!("has an unexpected key {k:?}"))),
        None => Ok(()),
    }
}

fn children<'a>(obj: &'a Map<String, Value>, at: &str) -> Result<&'a [Value]> {
    match obj.get("content") {
        None => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => Err(bad(at, "\"content\" is not an array")),
    }
}

/// The node's attrs, checking that only `declared` names appear.
fn attrs<'a>(
    obj: &'a Map<String, Value>,
    declared: &[&str],
    at: &str,
) -> Result<Option<&'a Map<String, Value>>> {
    match obj.get("attrs") {
        None => Ok(None),
        Some(Value::Object(a)) => {
            if let Some(k) = a.keys().find(|k| !declared.contains(&k.as_str())) {
                return Err(bad(at, format!("has an undeclared attribute {k:?}")));
            }
            Ok(Some(a))
        }
        Some(_) => Err(bad(at, "\"attrs\" is not an object")),
    }
}

fn check_align(a: Option<&Map<String, Value>>, at: &str) -> Result<()> {
    if let Some(v) = a.and_then(|a| a.get("align")) {
        if !v.as_str().is_some_and(|s| ALIGNS.contains(&s)) {
            return Err(bad(at, format!("align {v} is not one of {ALIGNS:?}")));
        }
    }
    Ok(())
}

fn check_block(v: &Value, at: &str) -> Result<()> {
    let obj = node_object(v, at)?;
    match node_type(obj, at)? {
        "paragraph" => {
            only_keys(obj, &["type", "attrs", "content"], at)?;
            check_align(attrs(obj, &["align"], at)?, at)?;
            check_inline_content(obj, at)
        }
        "heading" => {
            only_keys(obj, &["type", "attrs", "content"], at)?;
            let a = attrs(obj, &["level", "align"], at)?;
            check_align(a, at)?;
            if let Some(level) = a.and_then(|a| a.get("level")) {
                if !level.as_u64().is_some_and(|l| (1..=3).contains(&l)) {
                    return Err(bad(at, format!("heading level {level} is not 1, 2 or 3")));
                }
            }
            check_inline_content(obj, at)
        }
        kind @ ("bullet_list" | "ordered_list") => {
            only_keys(obj, &["type", "attrs", "content"], at)?;
            if kind == "ordered_list" {
                if let Some(order) = attrs(obj, &["order"], at)?.and_then(|a| a.get("order")) {
                    if order.as_u64().is_none() {
                        return Err(bad(at, format!("order {order} is not a whole number")));
                    }
                }
            } else {
                attrs(obj, &[], at)?;
            }
            let items = children(obj, at)?;
            if items.is_empty() {
                return Err(bad(at, "a list needs at least one list_item"));
            }
            for (i, item) in items.iter().enumerate() {
                check_list_item(item, &format!("{at}.content[{i}]"))?;
            }
            Ok(())
        }
        "list_item" => Err(bad(at, "a list_item must be inside a list")),
        "text" | "numFmt" | "notebook_link" => Err(bad(
            at,
            "inline content must be inside a paragraph or heading",
        )),
        other => Err(bad(at, format!("{other:?} is not a node DunneNote knows"))),
    }
}

fn check_list_item(v: &Value, at: &str) -> Result<()> {
    let obj = node_object(v, at)?;
    expect_type(obj, "list_item", at)?;
    only_keys(obj, &["type", "attrs", "content"], at)?;
    attrs(obj, &[], at)?;
    let content = children(obj, at)?;
    let first_is_paragraph = content
        .first()
        .and_then(Value::as_object)
        .and_then(|o| o.get("type"))
        .and_then(Value::as_str)
        == Some("paragraph");
    if !first_is_paragraph {
        return Err(bad(at, "a list_item must start with a paragraph"));
    }
    for (i, b) in content.iter().enumerate() {
        check_block(b, &format!("{at}.content[{i}]"))?;
    }
    Ok(())
}

fn check_inline_content(obj: &Map<String, Value>, at: &str) -> Result<()> {
    for (i, n) in children(obj, at)?.iter().enumerate() {
        check_inline(n, &format!("{at}.content[{i}]"))?;
    }
    Ok(())
}

fn check_inline(v: &Value, at: &str) -> Result<()> {
    let obj = node_object(v, at)?;
    match node_type(obj, at)? {
        "text" => {
            only_keys(obj, &["type", "marks", "text"], at)?;
            match obj.get("text") {
                Some(Value::String(s)) if !s.is_empty() => {}
                Some(Value::String(_)) => return Err(bad(at, "text nodes must not be empty")),
                _ => return Err(bad(at, "a text node needs a \"text\" string")),
            }
        }
        "numFmt" => {
            only_keys(obj, &["type", "attrs", "marks"], at)?;
            if let Some(a) = attrs(obj, &["raw", "format"], at)? {
                if let Some(raw) = a.get("raw") {
                    if !raw.is_string() {
                        return Err(bad(at, "numFmt raw is not a string"));
                    }
                }
                if let Some(f) = a.get("format") {
                    check_number_format(f, at)?;
                }
            }
        }
        "notebook_link" => {
            only_keys(obj, &["type", "attrs", "marks"], at)?;
            let declared = [
                "canvasId",
                "labelSnapshot",
                "notebookId",
                "notebookLabelSnapshot",
            ];
            if let Some(a) = attrs(obj, &declared, at)? {
                if let Some((k, _)) = a.iter().find(|(_, v)| !v.is_string()) {
                    return Err(bad(at, format!("notebook_link {k} is not a string")));
                }
            }
        }
        other => {
            return Err(bad(
                at,
                format!("{other:?} cannot appear inside a paragraph or heading"),
            ))
        }
    }
    check_marks(obj.get("marks"), at)
}

fn check_number_format(f: &Value, at: &str) -> Result<()> {
    let f = f
        .as_object()
        .ok_or_else(|| bad(at, "numFmt format is not an object"))?;
    let allowed = [
        "style",
        "locale",
        "currency",
        "currencyDisplay",
        "minimumFractionDigits",
        "maximumFractionDigits",
        "useGrouping",
        "negativeStyle",
    ];
    only_keys(f, &allowed, &format!("{at} numFmt format"))?;
    let one_of = |key: &str, values: &[&str]| -> Result<()> {
        match f.get(key) {
            None => Ok(()),
            Some(v) if v.as_str().is_some_and(|s| values.contains(&s)) => Ok(()),
            Some(v) => Err(bad(
                at,
                format!("numFmt {key} {v} is not one of {values:?}"),
            )),
        }
    };
    if f.get("style").is_none() || !f.get("locale").is_some_and(|l| l.is_string()) {
        return Err(bad(at, "numFmt format needs a style and a locale"));
    }
    one_of("style", &["decimal", "currency", "percent"])?;
    one_of("currencyDisplay", &["symbol", "code"])?;
    one_of("negativeStyle", &["minus", "parens"])?;
    if let Some(c) = f.get("currency") {
        if !c
            .as_str()
            .is_some_and(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase()))
        {
            return Err(bad(at, "numFmt currency is not a 3-letter ISO code"));
        }
    }
    for key in ["minimumFractionDigits", "maximumFractionDigits"] {
        if let Some(v) = f.get(key) {
            if v.as_u64().is_none_or(|d| d > 20) {
                return Err(bad(at, format!("numFmt {key} is not 0..20")));
            }
        }
    }
    if f.get("useGrouping").is_some_and(|g| !g.is_boolean()) {
        return Err(bad(at, "numFmt useGrouping is not true or false"));
    }
    Ok(())
}

fn check_marks(marks: Option<&Value>, at: &str) -> Result<()> {
    let marks = match marks {
        None => return Ok(()),
        Some(Value::Array(m)) => m,
        Some(_) => return Err(bad(at, "\"marks\" is not an array")),
    };
    let mut seen = Vec::new();
    for (i, m) in marks.iter().enumerate() {
        let at = format!("{at}.marks[{i}]");
        let obj = node_object(m, &at)?;
        only_keys(obj, &["type", "attrs"], &at)?;
        let t = node_type(obj, &at)?;
        if !MARKS.contains(&t) {
            return Err(bad(&at, format!("{t:?} is not a mark DunneNote knows")));
        }
        if seen.contains(&t) {
            return Err(bad(&at, format!("mark {t:?} appears twice")));
        }
        seen.push(t);
        let required = |a: Option<&Map<String, Value>>, key: &str| -> Result<Value> {
            a.and_then(|a| a.get(key))
                .cloned()
                .ok_or_else(|| bad(&at, format!("{t} needs attribute {key:?}")))
        };
        match t {
            "strong" | "em" | "underline" | "strike" => {
                attrs(obj, &[], &at)?;
            }
            "link" => {
                let a = attrs(obj, &["href", "title"], &at)?;
                let href = required(a, "href")?;
                if !href.as_str().is_some_and(safe_href) {
                    return Err(bad(&at, format!("link href {href} is not a safe link")));
                }
                if a.and_then(|a| a.get("title"))
                    .is_some_and(|t| !(t.is_null() || t.is_string()))
                {
                    return Err(bad(&at, "link title is not a string or null"));
                }
            }
            "font_family" => {
                let key = required(attrs(obj, &["key"], &at)?, "key")?;
                if !key.as_str().is_some_and(|k| FONT_KEYS.contains(&k)) {
                    return Err(bad(
                        &at,
                        format!("font key {key} is not one of {FONT_KEYS:?}"),
                    ));
                }
            }
            "font_size" => {
                let px = required(attrs(obj, &["px"], &at)?, "px")?;
                if !px.as_u64().is_some_and(|p| (1..=1000).contains(&p)) {
                    return Err(bad(
                        &at,
                        format!("font size {px} is not a whole number of px"),
                    ));
                }
            }
            _ => {
                let color = required(attrs(obj, &["color"], &at)?, "color")?;
                if !color.as_str().is_some_and(is_canonical_color) {
                    return Err(bad(
                        &at,
                        format!("color {color} is not #rrggbb, #rrggbbaa, rgb(r, g, b) or rgba(r, g, b, a)"),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Link targets DunneNote will follow: no script-bearing schemes, no control characters.
pub fn safe_href(href: &str) -> bool {
    let trimmed = href.trim();
    if trimmed.is_empty() || href.chars().any(char::is_control) {
        return false;
    }
    let scheme: String = trimmed
        .chars()
        .take_while(|c| *c != ':')
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    !(trimmed.contains(':')
        && ["javascript", "data", "vbscript", "blob"].contains(&scheme.as_str()))
}

/// Colors in the canonical form DunneNote stores: lowercase `#rrggbb` / `#rrggbbaa`,
/// `rgb(r, g, b)` or `rgba(r, g, b, a)` with channels 0–255 and alpha 0–1.
pub fn is_canonical_color(c: &str) -> bool {
    if let Some(hex) = c.strip_prefix('#') {
        return (hex.len() == 6 || hex.len() == 8)
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    }
    let channel = |s: &str| {
        !s.is_empty()
            && s.len() <= 3
            && s.bytes().all(|b| b.is_ascii_digit())
            && (s == "0" || !s.starts_with('0'))
            && s.parse::<u16>().is_ok_and(|n| n <= 255)
    };
    if let Some(body) = c.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = body.split(", ").collect();
        return parts.len() == 3 && parts.iter().all(|p| channel(p));
    }
    if let Some(body) = c.strip_prefix("rgba(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = body.split(", ").collect();
        return parts.len() == 4
            && parts[..3].iter().all(|p| channel(p))
            && parts[3]
                .parse::<f64>()
                .is_ok_and(|a| (0.0..=1.0).contains(&a) && js_number(a) == parts[3]);
    }
    false
}

// ---- building rich text ---------------------------------------------------------------------

/// A text node, with marks given as mark JSON (e.g. `json!({"type":"strong"})`).
pub fn text(s: &str, marks: Vec<Value>) -> Value {
    if marks.is_empty() {
        json!({"type": "text", "text": s})
    } else {
        json!({"type": "text", "marks": marks, "text": s})
    }
}

/// A left-aligned paragraph, in the JSON DunneNote's editor produces.
pub fn paragraph(content: Vec<Value>) -> Value {
    if content.is_empty() {
        json!({"type": "paragraph", "attrs": {"align": "left"}})
    } else {
        json!({"type": "paragraph", "attrs": {"align": "left"}, "content": content})
    }
}

/// A left-aligned heading (level 1–3).
pub fn heading(level: u8, content: Vec<Value>) -> Value {
    let level = level.clamp(1, 3);
    if content.is_empty() {
        json!({"type": "heading", "attrs": {"level": level, "align": "left"}})
    } else {
        json!({"type": "heading", "attrs": {"level": level, "align": "left"}, "content": content})
    }
}

/// A document from blocks; an empty list gives one empty paragraph.
pub fn doc(blocks: Vec<Value>) -> Value {
    if blocks.is_empty() {
        return json!({"type": "doc", "content": [paragraph(Vec::new())]});
    }
    json!({"type": "doc", "content": blocks})
}

/// A rich text document from plain text: one paragraph per line.
pub fn rich_text_from_plain(s: &str) -> Value {
    doc(s
        .lines()
        .map(|line| {
            let line = line.trim_end();
            paragraph(if line.is_empty() {
                Vec::new()
            } else {
                vec![text(line, Vec::new())]
            })
        })
        .collect())
}

/// A rich text document from a small, common subset of Markdown: `#`–`######` headings (4–6
/// become level 3), paragraphs, `-`/`*`/`+` and `1.` lists (one level), and inline `**bold**`,
/// `*italic*`/`_italic_`, `~~strike~~` and `[links](https://…)`. Anything else is kept as text.
pub fn rich_text_from_markdown(md: &str) -> Value {
    let mut blocks = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut list: Option<(bool, u64, Vec<Value>)> = None; // (ordered, start, items)

    fn flush_para(para: &mut Vec<&str>, blocks: &mut Vec<Value>) {
        if !para.is_empty() {
            blocks.push(paragraph(inline_markdown(&para.join(" "))));
            para.clear();
        }
    }
    fn flush_list(list: &mut Option<(bool, u64, Vec<Value>)>, blocks: &mut Vec<Value>) {
        if let Some((ordered, start, items)) = list.take() {
            blocks.push(if ordered {
                json!({"type": "ordered_list", "attrs": {"order": start}, "content": items})
            } else {
                json!({"type": "bullet_list", "content": items})
            });
        }
    }

    for raw in md.lines() {
        let line = raw.trim();
        if line.is_empty() {
            flush_para(&mut para, &mut blocks);
            flush_list(&mut list, &mut blocks);
            continue;
        }
        let hashes = line.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            flush_para(&mut para, &mut blocks);
            flush_list(&mut list, &mut blocks);
            blocks.push(heading(
                hashes.min(3) as u8,
                inline_markdown(line[hashes..].trim()),
            ));
            continue;
        }
        let bullet = ["- ", "* ", "+ "].iter().find_map(|m| line.strip_prefix(m));
        let numbered = line.split_once(". ").and_then(|(n, rest)| {
            (!n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()))
                .then(|| (n.parse::<u64>().unwrap_or(1), rest))
        });
        let item = match (bullet, numbered) {
            (Some(rest), _) => Some((false, 1, rest)),
            (None, Some((n, rest))) => Some((true, n, rest)),
            _ => None,
        };
        if let Some((ordered, start, rest)) = item {
            flush_para(&mut para, &mut blocks);
            if list.as_ref().is_some_and(|(o, _, _)| *o != ordered) {
                flush_list(&mut list, &mut blocks);
            }
            let entry = list.get_or_insert_with(|| (ordered, start, Vec::new()));
            entry.2.push(
                json!({"type": "list_item", "content": [paragraph(inline_markdown(rest.trim()))]}),
            );
            continue;
        }
        flush_list(&mut list, &mut blocks);
        para.push(line);
    }
    flush_para(&mut para, &mut blocks);
    flush_list(&mut list, &mut blocks);
    doc(blocks)
}

/// Inline Markdown to text nodes. Unmatched markers stay literal text.
fn inline_markdown(s: &str) -> Vec<Value> {
    #[derive(Clone, Copy, PartialEq)]
    enum M {
        Strong,
        Em,
        Strike,
    }
    let mut out: Vec<Value> = Vec::new();
    let mut active: Vec<M> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = s.chars().collect();

    let marks_json = |active: &[M], link: Option<&str>| -> Vec<Value> {
        let mut m = Vec::new();
        for (kind, name) in [(M::Strong, "strong"), (M::Em, "em"), (M::Strike, "strike")] {
            if active.contains(&kind) {
                m.push(json!({"type": name}));
            }
        }
        if let Some(href) = link {
            m.push(json!({"type": "link", "attrs": {"href": href, "title": null}}));
        }
        m
    };
    let flush = |buf: &mut String, out: &mut Vec<Value>, active: &[M]| {
        if !buf.is_empty() {
            out.push(text(buf, marks_json(active, None)));
            buf.clear();
        }
    };
    let closes_later = |from: usize, marker: &str| -> bool {
        let rest: String = chars[from..].iter().collect();
        rest.contains(marker)
    };

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let (kind, len) = match (c, next) {
            ('*', Some('*')) => (Some(M::Strong), 2),
            ('~', Some('~')) => (Some(M::Strike), 2),
            ('*', _) | ('_', _) => (Some(M::Em), 1),
            _ => (None, 1),
        };
        if let Some(kind) = kind {
            let marker: String = chars[i..i + len].iter().collect();
            if active.contains(&kind) {
                flush(&mut buf, &mut out, &active);
                active.retain(|k| *k != kind);
                i += len;
                continue;
            }
            // A single `_` inside a word (snake_case) is text, as in CommonMark.
            let intraword_underscore = c == '_'
                && i > 0
                && chars[i - 1].is_alphanumeric()
                && next.is_some_and(char::is_alphanumeric);
            if !intraword_underscore && closes_later(i + len, &marker) {
                flush(&mut buf, &mut out, &active);
                active.push(kind);
                i += len;
                continue;
            }
        }
        if c == '[' {
            let rest: String = chars[i + 1..].iter().collect();
            if let Some((label, after)) = rest.split_once("](") {
                if let Some((href, _)) = after.split_once(')') {
                    if !label.is_empty() && !label.contains('[') && safe_href(href) {
                        flush(&mut buf, &mut out, &active);
                        out.push(text(label, marks_json(&active, Some(href))));
                        i += 1 + label.chars().count() + 2 + href.chars().count() + 1;
                        continue;
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(&mut buf, &mut out, &active);
    out
}

// ---- sketches -------------------------------------------------------------------------------

/// Check strokes the way DunneNote reads them: a stroke it would drop (fewer than two points, a
/// non-finite coordinate, an empty id or colour, a non-positive width) is an error here, because
/// DunneNote opens a sketch with dropped strokes read-only.
pub fn check_strokes(strokes: &[Stroke]) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    for (i, s) in strokes.iter().enumerate() {
        let at = format!("stroke {i}");
        if s.id.is_empty() || !ids.insert(s.id.as_str()) {
            return Err(Error::Invalid(format!("{at}: id is empty or repeated")));
        }
        if s.points.len() < 2 {
            return Err(Error::Invalid(format!("{at}: needs at least two points")));
        }
        if s.points
            .iter()
            .any(|p| !p.x.is_finite() || !p.y.is_finite())
        {
            return Err(Error::Invalid(format!(
                "{at}: a coordinate is not a number"
            )));
        }
        if s.color.is_empty() {
            return Err(Error::Invalid(format!("{at}: colour is empty")));
        }
        if !(s.width.is_finite() && s.width > 0.0 && s.width <= 1000.0) {
            return Err(Error::Invalid(format!("{at}: width is not in (0, 1000]")));
        }
    }
    Ok(())
}

/// The stored JSON for `strokes`, byte-identical to DunneNote's own serializer.
pub fn sketch_json(strokes: &[Stroke]) -> Result<String> {
    check_strokes(strokes)?;
    let mut out = String::from(r#"{"v":1,"strokes":["#);
    for (i, s) in strokes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(r#"{"id":"#);
        out.push_str(&Value::String(s.id.clone()).to_string());
        out.push_str(r#","points":["#);
        for (j, p) in s.points.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            let x = round(p.x.clamp(0.0, 1.0), 4);
            let y = round(p.y.clamp(0.0, 1.0), 4);
            let pressure = round(clamp_pressure(p.p).max(0.01), 2);
            out.push_str(&format!(
                r#"{{"x":{},"y":{},"p":{}}}"#,
                js_number(x),
                js_number(y),
                js_number(pressure)
            ));
        }
        out.push_str(r#"],"color":"#);
        out.push_str(&Value::String(s.color.clone()).to_string());
        out.push_str(&format!(
            r#","width":{},"tool":"pen"}}"#,
            js_number(s.width)
        ));
    }
    out.push_str("]}");
    if out.len() > MAX_DOC_BYTES {
        return Err(Error::Invalid(format!(
            "the sketch is {} bytes; DunneNote stores at most {MAX_DOC_BYTES}",
            out.len()
        )));
    }
    Ok(out)
}

/// Pressure as DunneNote reads it: no usable value means 0.5; above 1 means 1.
fn clamp_pressure(p: f64) -> f64 {
    if !p.is_finite() || p <= 0.0 {
        0.5
    } else {
        p.min(1.0)
    }
}

/// `Math.round(n * 10^dp) / 10^dp` for the non-negative values used here.
fn round(n: f64, dp: i32) -> f64 {
    let f = 10f64.powi(dp);
    (n * f).round() / f
}

/// A number as JavaScript's `JSON.stringify` writes it, for finite values in the ranges used
/// here (no exponent form below 1e21 or above 1e-7): `1` not `1.0`, `0` not `-0`.
fn js_number(n: f64) -> String {
    format!("{}", n + 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StrokePoint;

    fn ok(v: Value) {
        check_rich_text(&v).unwrap_or_else(|e| panic!("{v} should pass: {e}"));
    }
    fn err(v: Value) {
        assert!(check_rich_text(&v).is_err(), "{v} should fail");
    }

    #[test]
    fn accepts_what_dunnenote_writes() {
        ok(serde_json::from_str(EMPTY_RICH_TEXT).unwrap());
        ok(json!({"type":"doc","content":[{"type":"heading","attrs":{"level":1}}]}));
        ok(json!({"type":"doc","content":[
            {"type":"paragraph","attrs":{"align":"center"},"content":[
                {"type":"text","marks":[{"type":"strong"},{"type":"link","attrs":{"href":"https://example.com","title":null}}],"text":"hi"},
                {"type":"numFmt","attrs":{"raw":"1234.5","format":{"style":"currency","locale":"en-US","currency":"USD"}}},
                {"type":"notebook_link","attrs":{"canvasId":"01920000-0000-7000-8000-00000000000a","labelSnapshot":"Plan","notebookId":"","notebookLabelSnapshot":""}}
            ]},
            {"type":"ordered_list","attrs":{"order":3},"content":[
                {"type":"list_item","content":[{"type":"paragraph"},{"type":"bullet_list","content":[
                    {"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"text_color","attrs":{"color":"rgba(1, 2, 3, 0.5)"}}],"text":"x"}]}]}]}]}
            ]}
        ]}));
    }

    #[test]
    fn rejects_what_dunnenote_would_refuse_or_alter() {
        err(json!({"type":"doc","content":[]}));
        err(json!({"type":"doc","content":[{"type":"text","text":"loose"}]}));
        err(
            json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":""}]}]}),
        );
        err(json!({"type":"doc","content":[{"type":"blockquote"}]}));
        err(json!({"type":"doc","content":[{"type":"paragraph","attrs":{"indent":2}}]}));
        err(json!({"type":"doc","content":[{"type":"heading","attrs":{"level":4}}]}));
        err(
            json!({"type":"doc","content":[{"type":"bullet_list","content":[{"type":"list_item","content":[{"type":"heading"}]}]}]}),
        );
        err(
            json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"link"}],"text":"x"}]}]}),
        );
        err(
            json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"link","attrs":{"href":"javascript:alert(1)"}}],"text":"x"}]}]}),
        );
        err(
            json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"highlight","attrs":{"color":"#FFF"}}],"text":"x"}]}]}),
        );
        err(
            json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"em"},{"type":"em"}],"text":"x"}]}]}),
        );
    }

    #[test]
    fn colors() {
        for c in [
            "#aabbcc",
            "#aabbcc80",
            "rgb(0, 12, 255)",
            "rgba(0, 0, 0, 0.25)",
            "rgba(1, 1, 1, 1)",
        ] {
            assert!(is_canonical_color(c), "{c}");
        }
        for c in [
            "#ABC",
            "#abc",
            "rgb(0,0,0)",
            "rgb(256, 0, 0)",
            "rgb(01, 0, 0)",
            "red",
            "rgba(0, 0, 0, 1.0)",
        ] {
            assert!(!is_canonical_color(c), "{c}");
        }
    }

    #[test]
    fn markdown_subset() {
        let d = rich_text_from_markdown(
            "# Title\n\nSome **bold** and *it* and [a link](https://x.test).\n\n- one\n- two\n\n3. three\n#### deep\nsnake_case_name",
        );
        check_rich_text(&d).unwrap();
        let s = d.to_string();
        assert!(s.contains(r#"{"type":"heading","attrs":{"level":1,"align":"left"},"content":[{"type":"text","text":"Title"}]}"#));
        assert!(s.contains(r#"{"type":"text","marks":[{"type":"strong"}],"text":"bold"}"#));
        assert!(s.contains(r#"{"type":"text","marks":[{"type":"link","attrs":{"href":"https://x.test","title":null}}],"text":"a link"}"#));
        assert!(s.contains(r#"{"type":"ordered_list","attrs":{"order":3}"#));
        assert!(s.contains(r#""level":3"#));
        assert!(s.contains("snake_case_name"));
    }

    fn stroke(points: &[(f64, f64, f64)]) -> Stroke {
        Stroke {
            id: "s1".into(),
            points: points
                .iter()
                .map(|&(x, y, p)| StrokePoint { x, y, p })
                .collect(),
            color: "#111111".into(),
            width: 2.0,
            tool: "eraser".into(),
        }
    }

    #[test]
    fn sketch_bytes_match_dunnenote() {
        assert_eq!(sketch_json(&[]).unwrap(), EMPTY_SKETCH);
        let s = sketch_json(&[stroke(&[(0.123456, -0.2, 0.0), (1.5, 0.5, 0.004)])]).unwrap();
        assert_eq!(
            s,
            r##"{"v":1,"strokes":[{"id":"s1","points":[{"x":0.1235,"y":0,"p":0.5},{"x":1,"y":0.5,"p":0.01}],"color":"#111111","width":2,"tool":"pen"}]}"##
        );
    }

    #[test]
    fn strokes_dunnenote_would_drop_are_refused() {
        assert!(sketch_json(&[stroke(&[(0.1, 0.1, 0.5)])]).is_err());
        assert!(sketch_json(&[stroke(&[(f64::NAN, 0.1, 0.5), (0.2, 0.2, 0.5)])]).is_err());
    }
}
