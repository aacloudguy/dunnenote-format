//! Plain text from a rich-text document, following DunneNote's own copy-as-text rules: text
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
}
