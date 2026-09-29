//! Property tests: everything that reads untrusted input (payloads, imported files, stored
//! positions, names) is fed generated and mutated input. Parsers must refuse bad input with an
//! error and never panic, and what the library writes must pass its own checks.
//!
//! Each property runs 256 cases by default; set `PROPTEST_CASES` for a longer run.

use dunnenote_format::write::ArchiveReason;
use dunnenote_format::{
    fold::fold, ics::parse_ics, ingest, payload, position, verify, At, Frame, Notebook, Settings,
    Sketch, Stroke, StrokePoint, VerifyLevel,
};
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use tempfile::TempDir;
use unicode_normalization::UnicodeNormalization;

#[path = "support/written.rs"]
mod written;

#[path = "support/common.rs"]
mod common;

// ---- rich text ---------------------------------------------------------------------------------

/// Any JSON value, up to a few levels deep. Stored documents come from `serde_json`, which
/// refuses nesting deeper than 128, so shallow trees are what a checker can meet.
fn any_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        any::<f64>().prop_map(|f| json!(f)),
        ".{0,12}".prop_map(Value::String),
    ];
    leaf.prop_recursive(5, 64, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec((json_key(), inner), 0..6)
                .prop_map(|kv| Value::Object(kv.into_iter().collect())),
        ]
    })
}

/// Object keys, biased towards the ones the rich text checker looks at.
fn json_key() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(vec![
            "type", "content", "text", "marks", "attrs", "level", "align", "order", "href",
            "title", "color", "key", "px", "raw", "format", "canvasId",
        ])
        .prop_map(String::from),
        1 => "[a-zA-Z]{1,8}",
    ]
}

/// Documents shaped like DunneNote's: known node and mark names with arbitrary attributes.
fn doc_like() -> impl Strategy<Value = Value> {
    let node_type = prop::sample::select(vec![
        "paragraph",
        "heading",
        "bullet_list",
        "ordered_list",
        "list_item",
        "text",
        "numFmt",
        "notebook_link",
        "hard_break",
        "image",
    ]);
    let mark = (
        prop::sample::select(payload::MARKS.to_vec()),
        prop::option::of(any_json()),
    )
        .prop_map(|(t, attrs)| match attrs {
            Some(a) => json!({"type": t, "attrs": a}),
            None => json!({"type": t}),
        });
    let leaf = (
        node_type.clone(),
        prop::option::of(".{0,8}"),
        prop::collection::vec(mark, 0..3),
        prop::option::of(any_json()),
    )
        .prop_map(|(t, text, marks, attrs)| {
            let mut node = Map::new();
            node.insert("type".into(), json!(t));
            if let Some(s) = text {
                node.insert("text".into(), json!(s));
            }
            if !marks.is_empty() {
                node.insert("marks".into(), Value::Array(marks));
            }
            if let Some(a) = attrs {
                node.insert("attrs".into(), a);
            }
            Value::Object(node)
        });
    let tree = leaf.prop_recursive(4, 48, 5, move |inner| {
        (node_type.clone(), prop::collection::vec(inner, 0..5))
            .prop_map(|(t, content)| json!({"type": t, "content": content}))
    });
    prop::collection::vec(tree, 0..5).prop_map(|content| json!({"type": "doc", "content": content}))
}

/// Valid documents built from the library's own helpers.
fn valid_doc() -> impl Strategy<Value = Value> {
    let marks = prop::sample::subsequence(vec!["strong", "em", "underline", "strike"], 0..=4)
        .prop_map(|names| {
            names
                .into_iter()
                .map(|n| json!({"type": n}))
                .collect::<Vec<_>>()
        });
    let inline = ("[^\\x00]{1,16}", marks).prop_map(|(s, m)| payload::text(&s, m));
    let block = prop_oneof![
        prop::collection::vec(inline.clone(), 0..4).prop_map(payload::paragraph),
        (1u8..=3, prop::collection::vec(inline, 0..4)).prop_map(|(l, c)| payload::heading(l, c)),
    ];
    prop::collection::vec(block, 1..6).prop_map(payload::doc)
}

proptest! {
    #[test]
    fn rich_text_checker_never_panics_on_any_json(v in any_json()) {
        let _ = payload::check_rich_text(&v);
    }

    #[test]
    fn rich_text_checker_never_panics_on_doc_like_trees(v in doc_like()) {
        let _ = payload::check_rich_text(&v);
    }

    #[test]
    fn documents_built_from_the_helpers_pass(v in valid_doc()) {
        prop_assert!(payload::check_rich_text(&v).is_ok(), "{v}");
    }

    #[test]
    fn markdown_and_plain_text_always_give_valid_documents(s in "(?s).{0,200}") {
        let md = payload::rich_text_from_markdown(&s);
        prop_assert!(payload::check_rich_text(&md).is_ok(), "{s:?} -> {md}");
        let plain = payload::rich_text_from_plain(&s);
        prop_assert!(payload::check_rich_text(&plain).is_ok(), "{s:?} -> {plain}");
    }

    #[test]
    fn markdown_made_of_markup_characters_gives_valid_documents(
        s in "[-*_~#\\[\\]()1. a\n]{0,80}"
    ) {
        let md = payload::rich_text_from_markdown(&s);
        prop_assert!(payload::check_rich_text(&md).is_ok(), "{s:?} -> {md}");
    }

    #[test]
    fn script_bearing_links_are_never_safe(
        scheme in prop::sample::select(vec!["javascript", "data", "vbscript", "blob"]),
        upper in prop::collection::vec(any::<bool>(), 10),
        lead in "[ ]{0,3}",
        rest in ".{0,20}",
    ) {
        let cased: String = scheme
            .chars()
            .zip(upper.iter().cycle())
            .map(|(c, up)| if *up { c.to_ascii_uppercase() } else { c })
            .collect();
        let href = format!("{lead}{cased}:{rest}");
        prop_assert!(!payload::safe_href(&href), "{href:?}");
    }
}

// ---- sketches ----------------------------------------------------------------------------------

/// Strokes already in stored precision, so they survive a write and read unchanged.
fn valid_strokes() -> impl Strategy<Value = Vec<Stroke>> {
    let point = (0u32..=10_000, 0u32..=10_000, 1u32..=100).prop_map(|(x, y, p)| StrokePoint {
        x: f64::from(x) / 10_000.0,
        y: f64::from(y) / 10_000.0,
        p: f64::from(p) / 100.0,
    });
    let stroke = (
        prop::collection::vec(point, 2..12),
        "#[0-9a-f]{6}",
        1u32..=10_000,
    );
    prop::collection::vec(stroke, 0..8).prop_map(|strokes| {
        strokes
            .into_iter()
            .enumerate()
            .map(|(i, (points, color, width))| Stroke {
                id: format!("s{i}"),
                points,
                color,
                width: f64::from(width) / 10.0,
                tool: "pen".into(),
            })
            .collect()
    })
}

#[derive(Debug, Clone)]
enum Break {
    EmptyId,
    RepeatedId,
    OnePoint,
    NonFinite(f64),
    Width(f64),
    EmptyColor,
}

fn a_break() -> impl Strategy<Value = Break> {
    prop_oneof![
        Just(Break::EmptyId),
        Just(Break::RepeatedId),
        Just(Break::OnePoint),
        prop::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY])
            .prop_map(Break::NonFinite),
        prop::sample::select(vec![0.0, -1.0, 1000.5, f64::NAN, f64::INFINITY])
            .prop_map(Break::Width),
        Just(Break::EmptyColor),
    ]
}

proptest! {
    #[test]
    fn valid_strokes_round_trip_exactly(strokes in valid_strokes()) {
        payload::check_strokes(&strokes).unwrap();
        let stored = payload::sketch_json(&strokes).unwrap();
        let sketch = Sketch {
            data: serde_json::from_str(&stored).unwrap(),
            schema_version: 1,
            updated_at: 0,
        };
        prop_assert_eq!(sketch.strokes(), strokes);
        // Writing what was read gives the same bytes.
        prop_assert_eq!(payload::sketch_json(&sketch.strokes()).unwrap(), stored);
    }

    #[test]
    fn broken_strokes_are_refused(
        mut strokes in valid_strokes().prop_filter("needs two strokes", |s| s.len() >= 2),
        which in any::<prop::sample::Index>(),
        broken in a_break(),
    ) {
        let i = which.index(strokes.len());
        let s = &mut strokes[i];
        match broken {
            Break::EmptyId => s.id.clear(),
            Break::RepeatedId => s.id = if i == 0 { "s1".into() } else { "s0".into() },
            Break::OnePoint => s.points.truncate(1),
            Break::NonFinite(f) => s.points[0].x = f,
            Break::Width(w) => s.width = w,
            Break::EmptyColor => s.color.clear(),
        }
        prop_assert!(payload::check_strokes(&strokes).is_err());
        prop_assert!(payload::sketch_json(&strokes).is_err());
    }
}

// ---- imported tables ---------------------------------------------------------------------------

fn assert_table_invariants(t: &ingest::ParsedTable) -> Result<(), TestCaseError> {
    prop_assert!(t.columns.len() <= ingest::MAX_COLUMNS);
    prop_assert!(t.rows.len() <= ingest::MAX_ROWS);
    let keys: std::collections::HashSet<&str> =
        t.columns.iter().map(|c| c.col_key.as_str()).collect();
    prop_assert_eq!(keys.len(), t.columns.len(), "column keys are unique");
    for c in &t.columns {
        prop_assert!(
            !c.col_key.is_empty()
                && c.col_key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "column key {:?}",
            c.col_key
        );
    }
    for row in &t.rows {
        for (k, v) in row {
            prop_assert!(keys.contains(k.as_str()), "cell under unknown column {k}");
            prop_assert!(!v.is_object() && !v.is_array(), "cell {v} is not a scalar");
            if let Some(s) = v.as_str() {
                prop_assert!(s.len() <= ingest::MAX_CELL_BYTES);
            }
        }
    }
    Ok(())
}

proptest! {
    #[test]
    fn csv_import_never_panics_and_keeps_its_invariants(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(t) = ingest::parse_csv(&bytes) {
            assert_table_invariants(&t)?;
        }
    }

    #[test]
    fn csv_like_text_imports_with_its_invariants(s in "[a-z0-9,;\t\"\n .-]{0,300}") {
        if let Ok(t) = ingest::parse_csv(s.as_bytes()) {
            assert_table_invariants(&t)?;
        }
    }

    #[test]
    fn json_import_never_panics_and_keeps_its_invariants(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        if let Ok(t) = ingest::parse_json(&bytes) {
            assert_table_invariants(&t)?;
        }
    }

    #[test]
    fn any_json_document_imports_with_its_invariants(v in any_json()) {
        if let Ok(t) = ingest::parse_json(v.to_string().as_bytes()) {
            assert_table_invariants(&t)?;
        }
    }

    #[test]
    fn a_plain_text_csv_reads_back_cell_for_cell(
        width in 1usize..6,
        cells in prop::collection::vec("x[a-z]{0,5}", 0..60),
    ) {
        let header: Vec<String> = (0..width).map(|i| format!("h{i}")).collect();
        let rows: Vec<&[String]> = cells.chunks(width).filter(|r| r.len() == width).collect();
        let mut csv = header.join(",");
        for r in &rows {
            csv.push('\n');
            csv.push_str(&r.join(","));
        }
        let t = ingest::parse_csv(csv.as_bytes()).unwrap();
        prop_assert_eq!(t.columns.len(), width);
        prop_assert_eq!(t.rows.len(), rows.len());
        for (row, expected) in t.rows.iter().zip(&rows) {
            for (col, want) in t.columns.iter().zip(expected.iter()) {
                prop_assert_eq!(row.get(&col.col_key), Some(&json!(want)));
            }
        }
    }

    #[test]
    fn typed_cells_are_numbers_blanks_or_the_text(s in "[-0-9. a]{0,8}") {
        match ingest::cell_from_text(&s) {
            Value::Number(n) => {
                let t = s.trim();
                prop_assert!(t.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit() || b == b'.'));
                prop_assert!(n.as_f64().is_some_and(f64::is_finite));
            }
            Value::String(text) => prop_assert!(text == s || (text.is_empty() && s.trim().is_empty())),
            other => prop_assert!(false, "unexpected cell {other}"),
        }
    }
}

// ---- calendars ---------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Edit {
    Replace(prop::sample::Index, u8),
    Delete(prop::sample::Index, usize),
    Insert(prop::sample::Index, String),
}

fn an_edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        (any::<prop::sample::Index>(), any::<u8>()).prop_map(|(i, b)| Edit::Replace(i, b)),
        (any::<prop::sample::Index>(), 1usize..40).prop_map(|(i, n)| Edit::Delete(i, n)),
        (
            any::<prop::sample::Index>(),
            prop::sample::select(vec![
                "\r\n",
                ":",
                ";",
                "=",
                "BEGIN:VEVENT\r\n",
                "END:VEVENT\r\n",
                "DTSTART:",
                "DTEND:19700101T000000Z\r\n",
                "DURATION:-P999999W\r\n",
                "TZID=",
                "Z",
                "T",
                "99999999",
                "\r\n ",
                ",",
                "ATTENDEE:",
                "CATEGORIES:,,,\r\n",
            ])
            .prop_map(String::from),
        )
            .prop_map(|(i, s)| Edit::Insert(i, s)),
    ]
}

fn apply(mut bytes: Vec<u8>, edits: &[Edit]) -> Vec<u8> {
    for e in edits {
        if bytes.is_empty() {
            break;
        }
        match e {
            Edit::Replace(i, b) => {
                let at = i.index(bytes.len());
                bytes[at] = *b;
            }
            Edit::Delete(i, n) => {
                let at = i.index(bytes.len());
                let end = (at + n).min(bytes.len());
                bytes.drain(at..end);
            }
            Edit::Insert(i, s) => {
                let at = i.index(bytes.len() + 1);
                bytes.splice(at..at, s.bytes());
            }
        }
    }
    bytes
}

proptest! {
    #[test]
    fn ics_parser_never_panics_on_any_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = parse_ics(&bytes);
    }

    #[test]
    fn ics_parser_never_panics_on_mutated_calendars(edits in prop::collection::vec(an_edit(), 1..12)) {
        let bytes = apply(written::CALENDAR_ICS.as_bytes().to_vec(), &edits);
        if let Ok(cal) = parse_ics(&bytes) {
            for e in &cal.events {
                prop_assert!(e.end_utc >= e.start_utc, "{e:?}");
                prop_assert!(e.attendees.len() <= 512);
                prop_assert!(e.categories.len() <= 64);
                prop_assert!(e.attachments.len() <= 64);
            }
        }
    }
}

#[test]
fn oversized_calendars_are_refused_not_parsed() {
    let mut big = b"BEGIN:VCALENDAR\r\n".to_vec();
    big.resize(dunnenote_format::ics::MAX_ICS_BYTES + 1, b'x');
    assert!(parse_ics(&big).is_err());
}

// ---- fold v1 and positions ---------------------------------------------------------------------

proptest! {
    #[test]
    fn fold_is_idempotent(s in "\\PC{0,24}") {
        let once = fold(&s);
        prop_assert_eq!(fold(&once), once);
    }

    #[test]
    fn fold_ignores_normalization_form(s in "\\PC{0,24}") {
        let nfd: String = s.nfd().collect();
        let nfc: String = s.nfc().collect();
        prop_assert_eq!(fold(&nfd), fold(&s));
        prop_assert_eq!(fold(&nfc), fold(&s));
    }

    #[test]
    fn fold_of_ascii_is_its_lowercase(s in "[ -~]{0,32}") {
        prop_assert_eq!(fold(&s), s.to_ascii_lowercase());
    }

    #[test]
    fn positions_stay_strictly_ordered(ops in prop::collection::vec(any::<prop::sample::Index>(), 1..60)) {
        let mut siblings: Vec<String> = Vec::new();
        for op in ops {
            let at = op.index(siblings.len() + 1);
            let new = if siblings.is_empty() {
                position::first()
            } else if at == 0 {
                position::before(&siblings[0]).unwrap()
            } else if at == siblings.len() {
                position::after(&siblings[at - 1]).unwrap()
            } else {
                position::between(&siblings[at - 1], &siblings[at]).unwrap()
            };
            prop_assert!(position::parse(&new).is_ok());
            prop_assert!(new.len() <= position::MAX_LEN);
            if at > 0 {
                prop_assert!(siblings[at - 1] < new, "{} !< {}", siblings[at - 1], new);
            }
            if at < siblings.len() {
                prop_assert!(new < siblings[at], "{} !< {}", new, siblings[at]);
            }
            siblings.insert(at, new);
        }
    }

    #[test]
    fn position_parser_never_panics(s in ".{0,300}") {
        let _ = position::parse(&s);
    }
}

// ---- the writer, end to end --------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Op {
    AddPage,
    AddRichText(prop::sample::Index, String),
    AddSketch(prop::sample::Index, Vec<Stroke>),
    AddPicture(prop::sample::Index),
    Archive(prop::sample::Index),
    Retrieve(prop::sample::Index),
    Tag(prop::sample::Index, String),
    DeletePage(prop::sample::Index),
}

fn an_op() -> impl Strategy<Value = Op> {
    let i = any::<prop::sample::Index>;
    prop_oneof![
        2 => Just(Op::AddPage),
        3 => (i(), "(?s).{0,40}").prop_map(|(p, s)| Op::AddRichText(p, s)),
        2 => (i(), valid_strokes()).prop_map(|(p, s)| Op::AddSketch(p, s)),
        3 => i().prop_map(Op::AddPicture),
        1 => i().prop_map(Op::Archive),
        1 => i().prop_map(Op::Retrieve),
        1 => (i(), "[A-Za-z]{1,6}(/[A-Za-z]{1,6})?").prop_map(|(p, t)| Op::Tag(p, t)),
        1 => i().prop_map(Op::DeletePage),
    ]
}

fn writer_cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map_or(24, |n| (n / 10).max(1))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(writer_cases()))]

    /// Any sequence of writes leaves a notebook that verifies clean, with every blob's
    /// reference count equal to the canvases that use it, including after pages are deleted
    /// and their canvases cascade away.
    #[test]
    fn any_sequence_of_writes_leaves_a_clean_notebook(ops in prop::collection::vec(an_op(), 1..16)) {
        let dir = TempDir::new().unwrap();
        let (mut nb, root) = common::new_notebook(&dir, "Prop");
        let mut pages = vec![common::page_in(&mut nb, &root)];
        for op in ops {
            let pick = |i: &prop::sample::Index, pages: &[String]| pages[i.index(pages.len())].clone();
            let outcome = match &op {
                Op::AddPage => nb
                    .write(|w| w.add_page(&root, "P", At::End))
                    .map(|p| pages.push(p)),
                Op::AddRichText(i, s) => {
                    let page = pick(i, &pages);
                    let doc = payload::rich_text_from_markdown(s);
                    nb.write(|w| w.add_rich_text(&page, Frame::PAGE_TEXT, Some(&doc), &Settings::new()))
                        .map(drop)
                }
                Op::AddSketch(i, strokes) => {
                    let page = pick(i, &pages);
                    nb.write(|w| w.add_sketch(&page, Frame::new(0, 0, 100, 100), strokes, &Settings::new()))
                        .map(drop)
                }
                Op::AddPicture(i) => {
                    let page = pick(i, &pages);
                    nb.write(|w| w.add_picture(&page, (0, 0), None, written::PNG, &Settings::new()))
                        .map(drop)
                }
                Op::Archive(i) => {
                    let page = pick(i, &pages);
                    nb.write(|w| w.archive_node(&page, ArchiveReason::Superseded, None))
                }
                Op::Retrieve(i) => {
                    let page = pick(i, &pages);
                    nb.write(|w| w.retrieve_node(&page))
                }
                Op::Tag(i, name) => {
                    let page = pick(i, &pages);
                    nb.write(|w| {
                        let tag = w.tag(name)?;
                        w.apply_tag(&tag, "node", &page)
                    })
                    .map(drop)
                }
                Op::DeletePage(i) if pages.len() > 1 => {
                    let at = i.index(pages.len());
                    let page = pages.remove(at);
                    nb.write(|w| {
                        let tx = w.transaction();
                        tx.execute(
                            "DELETE FROM item_tags WHERE source_kind = 'node' AND source_id = ?1",
                            [&page],
                        )?;
                        tx.execute("DELETE FROM nodes WHERE id = ?1", [&page])?;
                        Ok(())
                    })
                }
                Op::DeletePage(_) => Ok(()),
            };
            // Archiving refuses an archived page (and retrieving a live one); every other
            // operation here is valid and must succeed.
            if !matches!(op, Op::Archive(_) | Op::Retrieve(_)) {
                prop_assert!(outcome.is_ok(), "{op:?}: {outcome:?}");
            }
        }
        let path = nb.root().to_path_buf();
        drop(nb);
        common::assert_clean(&path);
        let nb = Notebook::open(&path).unwrap();
        let drift: i64 = nb
            .connection()
            .query_row(
                "SELECT count(*) FROM blobs b WHERE b.refcount <> \
                 (SELECT count(*) FROM canvas_instances c WHERE c.source_hash = b.hash)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        prop_assert_eq!(drift, 0);
        let report = verify(&nb, VerifyLevel::Full).unwrap();
        prop_assert!(!report.has_errors(), "{:?}", report.findings);
    }
}
