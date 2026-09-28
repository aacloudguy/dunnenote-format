//! Text from a rich-text document (plain or Markdown), and UTC date formatting.
//!
//! Plain text, following DunneNote's own copy-as-text rules: text
//! nodes give their text, a number atom (`numFmt`) its raw value, a notebook link its label.

use serde_json::Value;

const BLOCKS: [&str; 6] = [
    "paragraph",
    "heading",
    "list_item",
    "bullet_list",
    "ordered_list",
    "doc",
];

/// The text of a ProseMirror document, one line per paragraph, heading or list item.
pub fn plain_text(doc: &Value) -> String {
    let mut out = String::new();
    push(doc, &mut out);
    let trimmed: Vec<&str> = out.lines().map(str::trim_end).collect();
    trimmed.join("\n").trim_matches('\n').to_string()
}

fn push(node: &Value, out: &mut String) {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "text" => out.push_str(node.get("text").and_then(Value::as_str).unwrap_or("")),
        "numFmt" => out.push_str(attr(node, "raw")),
        "notebook_link" => out.push_str(attr(node, "labelSnapshot")),
        "hard_break" => out.push('\n'),
        _ => {
            if let Some(children) = node.get("content").and_then(Value::as_array) {
                for child in children {
                    push(child, out);
                }
            }
            if BLOCKS.contains(&kind) && kind != "doc" && !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
}

fn attr<'a>(node: &'a Value, name: &str) -> &'a str {
    node.get("attrs")
        .and_then(|a| a.get(name))
        .and_then(Value::as_str)
        .unwrap_or("")
}

// ---- Markdown -----------------------------------------------------------------------------------

/// A rich-text document as CommonMark (with GitHub's `~~strike~~`).
///
/// Headings, paragraphs and (nested) lists map directly. Bold, italic, strike and links become
/// Markdown; underline becomes `<u>…</u>`; font, size, colour and highlight are dropped (the
/// text is kept). A number atom gives its raw value and a notebook link its label. Nodes this
/// function does not know keep their text as a paragraph, so nothing written is lost.
pub fn markdown(doc: &Value) -> String {
    markdown_with(doc, 0)
}

/// [`markdown`] with every heading moved `heading_offset` levels down (at most `######`), for
/// documents placed under a title of their own.
pub fn markdown_with(doc: &Value, heading_offset: u64) -> String {
    let mut blocks = Vec::new();
    for child in children(doc) {
        block_md(child, heading_offset, &mut blocks);
    }
    let mut out = blocks.join("\n\n");
    out.truncate(out.trim_end().len());
    out
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn block_md(node: &Value, offset: u64, out: &mut Vec<String>) {
    match node.get("type").and_then(Value::as_str).unwrap_or("") {
        "heading" => {
            let level = node
                .get("attrs")
                .and_then(|a| a.get("level"))
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .saturating_add(offset)
                .clamp(1, 6);
            let text = inline_md(children(node));
            if !text.is_empty() {
                out.push(format!("{} {text}", "#".repeat(level as usize)));
            }
        }
        "paragraph" => {
            let text = inline_md(children(node));
            if !text.is_empty() {
                out.push(escape_line_start(&text));
            }
        }
        "bullet_list" | "ordered_list" => {
            let text = list_md(node, offset);
            if !text.is_empty() {
                out.push(text);
            }
        }
        _ => {
            // Unknown block: keep its text.
            let text = escape_md(&plain_text(node));
            if !text.is_empty() {
                out.push(text);
            }
        }
    }
}

fn list_md(list: &Value, offset: u64) -> String {
    let ordered = list.get("type").and_then(Value::as_str) == Some("ordered_list");
    let mut n = list
        .get("attrs")
        .and_then(|a| a.get("order"))
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let mut lines = Vec::new();
    for item in children(list) {
        let marker = if ordered {
            let m = format!("{n}. ");
            n += 1;
            m
        } else {
            "- ".to_string()
        };
        let indent = " ".repeat(marker.len());
        let mut blocks = Vec::new();
        for child in children(item) {
            block_md(child, offset, &mut blocks);
        }
        let body = blocks.join("\n");
        let mut first = true;
        for line in body.lines() {
            if first {
                lines.push(format!("{marker}{line}"));
                first = false;
            } else if line.is_empty() {
                lines.push(String::new());
            } else {
                lines.push(format!("{indent}{line}"));
            }
        }
        if first {
            lines.push(marker.trim_end().to_string());
        }
    }
    lines.join("\n")
}

/// Mark names in nesting order (outermost first) and their Markdown delimiters.
const MARKS: [(&str, &str, &str); 4] = [
    ("strong", "**", "**"),
    ("em", "*", "*"),
    ("strike", "~~", "~~"),
    ("underline", "<u>", "</u>"),
];

/// One run of inline content: its text and the marks that change how Markdown renders it.
struct Run {
    text: String,
    marks: Vec<&'static str>,
    href: Option<String>,
}

fn run_of(node: &Value) -> Option<Run> {
    let text = match node.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => escape_md(node.get("text").and_then(Value::as_str).unwrap_or("")),
        "numFmt" => escape_md(attr(node, "raw")),
        "notebook_link" => escape_md(attr(node, "labelSnapshot")),
        "hard_break" => "\\\n".to_string(),
        _ => escape_md(&plain_text(node)),
    };
    let mut marks = Vec::new();
    let mut href = None;
    for mark in node
        .get("marks")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let kind = mark.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "link" {
            href = mark
                .get("attrs")
                .and_then(|a| a.get("href"))
                .and_then(Value::as_str)
                .map(str::to_string);
        } else if let Some((name, _, _)) = MARKS.iter().find(|(n, _, _)| *n == kind) {
            marks.push(*name);
        }
    }
    marks.sort_by_key(|m| MARKS.iter().position(|(n, _, _)| n == m));
    Some(Run { text, marks, href })
}

fn inline_md(nodes: &[Value]) -> String {
    // Merge neighbours that render the same, so `**a****b**` becomes `**ab**`.
    let mut runs: Vec<Run> = Vec::new();
    for run in nodes.iter().filter_map(run_of) {
        match runs.last_mut() {
            Some(last) if last.marks == run.marks && last.href == run.href => {
                last.text.push_str(&run.text);
            }
            _ => runs.push(run),
        }
    }
    let mut out = String::new();
    for run in runs {
        // Delimiters must hug the text: move surrounding spaces outside them.
        let core = run.text.trim();
        if core.is_empty() {
            out.push_str(&run.text);
            continue;
        }
        let lead = &run.text[..run.text.len() - run.text.trim_start().len()];
        let trail = &run.text[run.text.trim_end().len()..];
        let mut piece = core.to_string();
        for name in run.marks.iter().rev() {
            if let Some((_, open, close)) = MARKS.iter().find(|(n, _, _)| n == name) {
                piece = format!("{open}{piece}{close}");
            }
        }
        if let Some(href) = &run.href {
            piece = format!("[{piece}](<{}>)", href.replace(['<', '>'], ""));
        }
        out.push_str(lead);
        out.push_str(&piece);
        out.push_str(trail);
    }
    out
}

/// Backslash-escape characters that Markdown would otherwise interpret inline.
pub fn escape_md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '*' | '_' | '[' | ']' | '`' | '<' | '>' | '|' | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape a paragraph whose first characters would start a heading, quote or list.
fn escape_line_start(text: &str) -> String {
    let trimmed = text.trim_start();
    let starts_block = trimmed.starts_with(['#', '-', '+', '='])
        || trimmed
            .find(|c: char| !c.is_ascii_digit())
            .is_some_and(|i| i > 0 && trimmed[i..].starts_with(['.', ')']));
    if starts_block {
        let indent = &text[..text.len() - trimmed.len()];
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 {
            return format!("{indent}{}\\{}", &trimmed[..digits], &trimmed[digits..]);
        }
        return format!("{indent}\\{trimmed}");
    }
    text.to_string()
}

// ---- dates --------------------------------------------------------------------------------------

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DD` (UTC) for Unix seconds.
pub fn utc_date(secs: i64) -> String {
    let (y, m, d) = civil(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD HH:MMZ` (UTC) for Unix seconds.
pub fn utc_datetime(secs: i64) -> String {
    let s = secs.rem_euclid(86_400);
    format!("{} {:02}:{:02}Z", utc_date(secs), s / 3600, (s % 3600) / 60)
}

/// RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`) for Unix seconds.
pub fn rfc3339(secs: i64) -> String {
    let s = secs.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        utc_date(secs),
        s / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

/// RFC 3339 with a numeric offset (`YYYY-MM-DDTHH:MM:SS+HH:MM`) for Unix seconds, shown in the
/// zone `offset_minutes` east of UTC — the form of a form submission's `Submitted` time.
pub fn rfc3339_offset(secs: i64, offset_minutes: i32) -> String {
    let local = secs + i64::from(offset_minutes) * 60;
    let s = local.rem_euclid(86_400);
    let sign = if offset_minutes >= 0 { '+' } else { '-' };
    let off = offset_minutes.unsigned_abs();
    format!(
        "{}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        utc_date(local),
        s / 3600,
        (s % 3600) / 60,
        s % 60,
        off / 60,
        off % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paragraphs_lists_atoms_and_links() {
        let doc = json!({"type":"doc","content":[
            {"type":"heading","attrs":{"level":1},"content":[{"type":"text","text":"Plan"}]},
            {"type":"paragraph","content":[
                {"type":"text","text":"Cost "},
                {"type":"numFmt","attrs":{"raw":"1234.5","format":{}}},
                {"type":"text","text":" see ","marks":[{"type":"strong"}]},
                {"type":"notebook_link","attrs":{"canvasId":"x","labelSnapshot":"Budget"}}
            ]},
            {"type":"bullet_list","content":[
                {"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                {"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
            ]},
            {"type":"paragraph"}
        ]});
        assert_eq!(plain_text(&doc), "Plan\nCost 1234.5 see Budget\none\ntwo");
    }

    #[test]
    fn unknown_nodes_keep_their_text() {
        let doc = json!({"type":"doc","content":[{"type":"future_block","content":[{"type":"text","text":"kept"}]}]});
        assert_eq!(plain_text(&doc), "kept");
    }

    #[test]
    fn markdown_blocks_marks_and_lists() {
        let doc = json!({"type":"doc","content":[
            {"type":"heading","attrs":{"level":2,"align":"left"},"content":[{"type":"text","text":"Plan"}]},
            {"type":"paragraph","attrs":{"align":"left"},"content":[
                {"type":"text","text":"a "},
                {"type":"text","marks":[{"type":"strong"}],"text":"bold "},
                {"type":"text","marks":[{"type":"strong"}],"text":"run"},
                {"type":"text","text":", "},
                {"type":"text","marks":[{"type":"em"},{"type":"strong"}],"text":"both"},
                {"type":"text","text":" and "},
                {"type":"text","marks":[{"type":"link","attrs":{"href":"https://example.com/","title":null}}],"text":"a link"},
                {"type":"text","text":" costing 5*3 [sic]"}
            ]},
            {"type":"ordered_list","attrs":{"order":3},"content":[
                {"type":"list_item","content":[
                    {"type":"paragraph","attrs":{"align":"left"},"content":[{"type":"text","text":"three"}]},
                    {"type":"bullet_list","content":[
                        {"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}
                    ]}
                ]},
                {"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","text":"four"}]}]}
            ]},
            {"type":"paragraph","content":[{"type":"text","text":"# not a heading"}]},
            {"type":"paragraph","content":[{"type":"text","text":"2026. A year"}]},
            {"type":"paragraph"}
        ]});
        assert_eq!(
            markdown(&doc),
            "## Plan\n\n\
             a **bold run**, ***both*** and [a link](<https://example.com/>) costing 5\\*3 \\[sic\\]\n\n\
             3. three\n   - nested\n4. four\n\n\
             \\# not a heading\n\n\
             2026\\. A year"
        );
    }

    #[test]
    fn markdown_keeps_unknown_nodes_and_underline() {
        let doc = json!({"type":"doc","content":[
            {"type":"future_block","content":[{"type":"text","text":"kept"}]},
            {"type":"paragraph","content":[{"type":"text","marks":[{"type":"underline"},{"type":"text_color","attrs":{"color":"red"}}],"text":"u"}]}
        ]});
        assert_eq!(markdown(&doc), "kept\n\n<u>u</u>");
    }

    #[test]
    fn offset_times() {
        assert_eq!(
            rfc3339_offset(1_767_605_400, 0),
            "2026-01-05T09:30:00+00:00"
        );
        assert_eq!(
            rfc3339_offset(1_767_605_400, 60),
            "2026-01-05T10:30:00+01:00"
        );
        assert_eq!(
            rfc3339_offset(1_767_605_400, -330),
            "2026-01-05T04:00:00-05:30"
        );
        assert_eq!(
            rfc3339_offset(1_767_571_200, -60),
            "2026-01-04T23:00:00-01:00"
        );
    }

    #[test]
    fn dates() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_datetime(1_758_000_000), "2025-09-16 05:20Z");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        assert_eq!(rfc3339(1_767_625_200), "2026-01-05T15:00:00Z");
    }
}
