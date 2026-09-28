//! Notebooks written by this library, for conformance in both directions.
//!
//! `tests/write.rs` builds them and checks them with this library; the `emit_written` example
//! builds the same notebooks for DunneNote's own round-trip test, which opens each one in the app
//! and requires the app to read what this library recorded.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use dunnenote_format::ingest::TypeHint;
use dunnenote_format::settings_keys::{FormField, LabelDisplay, Placement};
use dunnenote_format::{
    payload, ArchiveReason, At, Frame, MetaValue, Notebook, Result, Settings, Stroke, StrokePoint,
    Submission,
};
use serde_json::{json, Value};

/// A 16x16 PNG with four opaque quadrants (the same image DunneNote's golden notebooks use).
pub const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0xF3, 0xFF,
    0x61, 0x00, 0x00, 0x00, 0x26, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x78, 0xE6, 0x65, 0xF3,
    0x1F, 0x1F, 0xFE, 0xB5, 0x0F, 0x3F, 0x66, 0x18, 0x35, 0x60, 0x58, 0x18, 0x60, 0xB3, 0xE0, 0xD9,
    0x7F, 0x7C, 0x98, 0x10, 0x18, 0x35, 0x60, 0x58, 0x18, 0x00, 0x00, 0x9A, 0x7B, 0x06, 0xEE, 0x0F,
    0xA8, 0xE5, 0xEB, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

pub const NOTES_MD: &str = "\
# Field notes

Collected on **Tuesday** with *two* sensors; see [the log](https://example.invalid/log).

- calibrate the probe
- record ~~three~~ four readings

1. upload
2. review
";

pub fn stroke(id: &str, points: &[(f64, f64, f64)], color: &str, width: f64) -> Stroke {
    Stroke {
        id: id.into(),
        points: points
            .iter()
            .map(|&(x, y, p)| StrokePoint { x, y, p })
            .collect(),
        color: color.into(),
        width,
        tool: "pen".into(),
    }
}

pub fn settings(v: Value) -> Settings {
    v.as_object().cloned().unwrap_or_default()
}

/// A document using every node and mark in DunneNote's editor schema.
pub fn every_node_doc(link_target: &str) -> Value {
    json!({"type": "doc", "content": [
        {"type": "heading", "attrs": {"level": 2, "align": "center"}, "content": [
            {"type": "text", "text": "Every node"}]},
        {"type": "paragraph", "attrs": {"align": "justify"}, "content": [
            {"type": "text", "marks": [{"type": "strong"}, {"type": "em"}, {"type": "underline"}, {"type": "strike"}], "text": "marked "},
            {"type": "text", "marks": [{"type": "font_family", "attrs": {"key": "literata"}}, {"type": "font_size", "attrs": {"px": 18}}], "text": "serif "},
            {"type": "text", "marks": [{"type": "text_color", "attrs": {"color": "#c0392b"}}, {"type": "highlight", "attrs": {"color": "rgba(255, 235, 59, 0.5)"}}], "text": "coloured "},
            {"type": "numFmt", "attrs": {"raw": "1234.5", "format": {"style": "currency", "locale": "en-US", "currency": "USD"}}},
            {"type": "text", "text": " "},
            {"type": "notebook_link", "attrs": {"canvasId": link_target, "labelSnapshot": "Field notes", "notebookId": "", "notebookLabelSnapshot": ""}}
        ]},
        {"type": "ordered_list", "attrs": {"order": 3}, "content": [
            {"type": "list_item", "content": [
                {"type": "paragraph", "attrs": {"align": "left"}, "content": [{"type": "text", "text": "third"}]},
                {"type": "bullet_list", "content": [
                    {"type": "list_item", "content": [{"type": "paragraph", "attrs": {"align": "left"}, "content": [{"type": "text", "text": "nested"}]}]}
                ]}
            ]}
        ]},
        {"type": "paragraph", "attrs": {"align": "left"}}
    ]})
}

/// Root → one section → one page with the text box DunneNote puts on a new page.
pub fn minimal(root: &Path) -> Result<()> {
    let mut nb = Notebook::create(root, Some("Written Minimal"))?;
    let nb_id = nb.notebook_node()?.id;
    nb.write(|w| {
        let section = w.add_section(&nb_id, "Inbox", At::End)?;
        let page = w.add_page(&section, "First page", At::End)?;
        w.add_rich_text(&page, Frame::PAGE_TEXT, None, &Settings::new())?;
        Ok(())
    })
}

/// Every M3 write: sibling placement, nesting, rich text (Markdown and every node and mark),
/// a sketch, a picture with alt text, markup and a caption, nested groups, a settings merge that
/// keeps an unknown key, a moved canvas and a rename. Several separate write transactions.
pub fn every_kind(root: &Path) -> Result<()> {
    let mut nb = Notebook::create(root, Some("Written Every Kind"))?;
    let nb_id = nb.notebook_node()?.id;
    let overview = nb.write(|w| {
        let research = w.add_section(&nb_id, "Research", At::End)?;
        let planning = w.add_section(&nb_id, "Planning", At::End)?;
        w.add_section(&nb_id, "Archive later", At::Start)?;
        let overview = w.add_page(&research, "Overview", At::End)?;
        let sources = w.add_page(&research, "Sources", At::End)?;
        w.add_page(&research, "Between", At::Before(&sources))?;
        w.add_page(&research, "After overview", At::After(&overview))?;
        let nested = w.add_section(&research, "Background", At::End)?;
        w.add_page(&nested, "Reading list", At::End)?;
        w.add_page(&planning, "Schedule", At::End)?;
        w.rename(&planning, "Plans")?;
        Ok(overview)
    })?;
    nb.write(|w| {
        let notes = w.add_rich_text(
            &overview,
            Frame::PAGE_TEXT,
            Some(&payload::rich_text_from_markdown(NOTES_MD)),
            &Settings::new(),
        )?;
        w.add_rich_text(
            &overview,
            Frame::new(40, 380, 560, 260),
            Some(&every_node_doc(&notes)),
            &settings(json!({"frameOutlineHidden": true})),
        )?;
        let sketch = w.add_sketch(
            &overview,
            Frame::new(620, 40, 400, 300),
            &[
                stroke(
                    "s1",
                    &[(0.1, 0.1, 0.5), (0.4, 0.35, 0.7), (0.8, 0.6, 1.0)],
                    "#2980b9",
                    3.0,
                ),
                stroke("s2", &[(0.2, 0.8, 0.25), (0.6, 0.8, 0.25)], "#c0392b", 2.0),
            ],
            &Settings::new(),
        )?;
        let picture = w.add_picture(
            &overview,
            (620, 380),
            None,
            PNG,
            &settings(json!({"alt": "Four coloured squares"})),
        )?;
        w.set_markup(
            &picture,
            &[stroke(
                "m1",
                &[(0.0, 0.0, 0.5), (1.0, 1.0, 0.5)],
                "#111111",
                2.0,
            )],
        )?;
        let caption = w.add_rich_text(
            &overview,
            Frame::new(620, 400, 160, 48),
            Some(&payload::rich_text_from_plain("Figure 1. Swatches")),
            &settings(json!({
                "backgroundTransparent": true,
                "caption": {"anchor": picture, "placement": "bottom"}
            })),
        )?;
        let outer = w.add_group(&overview, None, &[&sketch])?;
        w.add_group(&overview, Some(&outer), &[&picture, &caption])?;
        Ok(())
    })?;
    // Later edits in a separate transaction, as a script touching an existing notebook would.
    nb.write(|w| {
        let canvases: Vec<String> = w
            .transaction()
            .prepare("SELECT id FROM canvas_instances WHERE kind = 'sketch'")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let sketch = &canvases[0];
        w.merge_settings(sketch, &settings(json!({"futureKey": {"kept": true}})))?;
        w.merge_settings(sketch, &settings(json!({"backgroundTransparent": true})))?;
        w.set_frame(sketch, Frame::new(640, 60, 420, 320))?;
        w.set_sketch(
            sketch,
            &[stroke(
                "s3",
                &[(0.5, 0.5, 0.5), (0.9, 0.1, 0.9)],
                "#27ae60",
                5.0,
            )],
        )?;
        Ok(())
    })
}

/// A notebook DunneNote wrote (the `every-kind` golden notebook), changed by this library: a new
/// page with text, a sketch on an existing page, and an existing text canvas rewritten.
pub fn into_golden(root: &Path, golden: &Path) -> Result<()> {
    copy_dir(golden, root)?;
    let mut nb = Notebook::open_writable(root)?;
    let section = nb
        .walk()?
        .into_iter()
        .find(|(_, n)| n.name == "Research")
        .map(|(_, n)| n.id)
        .expect("the golden notebook has a Research section");
    let overview = nb
        .pages()?
        .into_iter()
        .find(|p| p.name == "Overview")
        .map(|p| p.id)
        .expect("the golden notebook has an Overview page");
    let text = nb
        .canvases(&overview)?
        .into_iter()
        .find(|c| c.kind == dunnenote_format::CanvasKind::RichText)
        .map(|c| c.id)
        .expect("Overview has rich text");
    nb.write(|w| {
        let page = w.add_page(&section, "Added by dunnenote-format", At::Start)?;
        w.add_rich_text(
            &page,
            Frame::PAGE_TEXT,
            Some(&payload::rich_text_from_markdown(
                "## Added\n\nWritten without DunneNote.",
            )),
            &Settings::new(),
        )?;
        w.add_sketch(
            &overview,
            Frame::new(40, 700, 300, 200),
            &[stroke(
                "g1",
                &[(0.1, 0.5, 0.5), (0.9, 0.5, 0.5)],
                "#8e44ad",
                3.0,
            )],
            &Settings::new(),
        )?;
        w.set_rich_text(
            &text,
            &payload::rich_text_from_markdown("Rewritten by **dunnenote-format**."),
        )?;
        Ok(())
    })
}

/// Tables: an imported CSV (numbers, booleans, dates, blanks) and JSON file, and a blank Editable
/// table edited every way, across separate transactions.
pub fn tables(root: &Path) -> Result<()> {
    let mut nb = Notebook::create(root, Some("Written Tables"))?;
    let nb_id = nb.notebook_node()?.id;
    let (page, sheet) = nb.write(|w| {
        let section = w.add_section(&nb_id, "Data", At::End)?;
        let page = w.add_page(&section, "Tables", At::End)?;
        w.add_table_from_csv(
            &page,
            Frame::new(40, 40, 480, 200),
            TABLE_CSV.as_bytes(),
            &Settings::new(),
        )?;
        w.add_table_from_json(
            &page,
            Frame::new(560, 40, 400, 200),
            TABLE_JSON.as_bytes(),
            &Settings::new(),
        )?;
        let sheet = w.add_table(&page, Frame::new(40, 280, 480, 360), &Settings::new())?;
        Ok((page, sheet))
    })?;
    let rows: Vec<String> = nb
        .dataset(&sheet)?
        .expect("a table has a dataset")
        .rows
        .into_iter()
        .map(|r| r.id)
        .collect();
    nb.write(|w| {
        w.rename_column(&sheet, "c0", "Name")?;
        w.rename_column(&sheet, "c1", "Hours")?;
        let notes = w.add_column(&sheet, "Notes", TypeHint::Text, None)?;
        w.set_cell(&sheet, &rows[0], "c0", &json!("Ada"))?;
        w.set_cell(&sheet, &rows[0], "c1", &json!(7.5))?;
        w.set_cell(&sheet, &rows[0], &notes, &json!("Ünïcode ✓"))?;
        w.set_cell(&sheet, &rows[1], "c0", &json!(""))?;
        w.delete_row(&sheet, &rows[2])?;
        let mut cells = serde_json::Map::new();
        cells.insert("c0".into(), json!("Grace"));
        cells.insert("c1".into(), json!(3));
        cells.insert(notes.clone(), json!(true));
        w.insert_row(&sheet, &cells)?;
        w.set_cell(&sheet, &rows[1], "c2", &json!("removed with its column"))?;
        w.delete_column(&sheet, "c2")?;
        w.move_column(&sheet, &notes, 0)?;
        Ok(())
    })?;
    nb.write(|w| {
        w.add_table(&page, Frame::new(560, 280, 300, 160), &Settings::new())?;
        Ok(())
    })
}

/// Forms: a new form with text, sketch and picture fields, two submissions (the second adding a
/// column to a table with rows), a hidden field, and captions in two placements.
pub fn forms(root: &Path) -> Result<()> {
    let mut nb = Notebook::create(root, Some("Written Forms"))?;
    let nb_id = nb.notebook_node()?.id;
    let (page, text) = nb.write(|w| {
        let section = w.add_section(&nb_id, "Intake", At::End)?;
        let (page, _) = w.add_form(&section, "Visit log", At::End)?;
        let text: String = w.transaction().query_row(
            "SELECT id FROM canvas_instances WHERE page_id = ?1 AND kind = 'rich_text'",
            [&page],
            |r| r.get(0),
        )?;
        Ok((page, text))
    })?;
    let (sig, photo) = nb.write(|w| {
        let mut visitor = FormField::new("Visitor");
        visitor.required = true;
        visitor.label_display = Some(LabelDisplay::Above);
        w.set_form_field(&text, Some(&visitor))?;
        w.set_rich_text(&text, &payload::rich_text_from_plain("Ada Lovelace"))?;
        let sig = w.add_sketch(
            &page,
            Frame::new(560, 40, 320, 120),
            &[stroke(
                "sig",
                &[(0.1, 0.6, 0.4), (0.5, 0.3, 0.8), (0.9, 0.6, 0.5)],
                "#111111",
                2.0,
            )],
            &Settings::new(),
        )?;
        let mut signature = FormField::new("Signature");
        signature.label = Some("Sign here".into());
        w.set_form_field(&sig, Some(&signature))?;
        let hidden = w.add_rich_text(
            &page,
            Frame::new(560, 200, 200, 40),
            Some(&payload::rich_text_from_plain("not asked")),
            &settings(json!({"hidden": true})),
        )?;
        w.set_form_field(&hidden, Some(&FormField::new("Hidden")))?;
        w.merge_page_settings(
            &page,
            &settings(json!({"formTabOrder": [sig, text], "formLabelDisplay": "below"})),
        )?;
        w.submit_form(
            &page,
            &Submission {
                submitted: Some("2026-01-05T09:30:00+00:00".into()),
                ..Submission::default()
            },
        )?;
        let photo = w.add_picture(&page, (560, 300), Some((160, 160)), PNG, &Settings::new())?;
        w.set_form_field(&photo, Some(&FormField::new("Photo")))?;
        Ok((sig, photo))
    })?;
    nb.write(|w| {
        w.set_rich_text(&text, &payload::rich_text_from_markdown("**Grace** Hopper"))?;
        w.set_sketch(&sig, &[])?;
        w.submit_form(
            &page,
            &Submission {
                submitted: Some("2026-01-06T17:05:09-05:00".into()),
                confirm_new_columns: true,
                ..Submission::default()
            },
        )?;
        w.add_caption(
            &photo,
            Some(&payload::rich_text_from_plain("Visitor photo")),
            Placement::Bottom,
        )?;
        let loose = w.add_picture(&page, (760, 300), Some((200, 120)), PNG, &Settings::new())?;
        w.add_caption(&loose, None, Placement::CornerBottomRight)?;
        Ok(())
    })
}

/// Templates: a page with every kind (text marked templateKeepContent, picture with a caption, a
/// table, a form's answers table), made into a template, and a page made from it.
pub fn templates(root: &Path) -> Result<()> {
    let mut nb = Notebook::create(root, Some("Written Templates"))?;
    let nb_id = nb.notebook_node()?.id;
    nb.write(|w| {
        let section = w.add_section(&nb_id, "Meetings", At::End)?;
        let (page, _) = w.add_form(&section, "Meeting notes", At::End)?;
        let heading = w.add_rich_text(
            &page,
            Frame::new(40, 380, 480, 60),
            Some(&payload::rich_text_from_markdown("## Weekly meeting")),
            &settings(json!({"templateKeepContent": true})),
        )?;
        let notes = w.add_rich_text(
            &page,
            Frame::new(40, 460, 480, 200),
            Some(&payload::rich_text_from_plain("Discussed the budget.")),
            &settings(json!({"scrollTopPx": 12})),
        )?;
        w.set_form_field(&notes, Some(&FormField::new("Notes")))?;
        let photo = w.add_picture(
            &page,
            (560, 40),
            Some((160, 160)),
            PNG,
            &settings(json!({"alt": "Room photo", "rotation": 90})),
        )?;
        w.add_caption(
            &photo,
            Some(&payload::rich_text_from_plain("The room")),
            Placement::Bottom,
        )?;
        let table = w.add_table(&page, Frame::new(560, 300, 300, 200), &Settings::new())?;
        w.add_column(&table, "Owner", TypeHint::Text, None)?;
        w.merge_page_settings(&page, &settings(json!({"formTabOrder": [notes, heading]})))?;
        let template = w.make_template(&page)?;
        w.new_from_template(&template, &section)?;
        Ok(())
    })
}

/// Archive and tags: pages and a section archived and one retrieved, an archived canvas, tags on
/// every kind with aliases, a rename and a merge, and typed metadata.
pub fn archive_tags(root: &Path) -> Result<()> {
    let mut nb = Notebook::create(root, Some("Written Archive and Tags"))?;
    let nb_id = nb.notebook_node()?.id;
    let (drafts, first, second, text, table) = nb.write(|w| {
        let drafts = w.add_section(&nb_id, "Drafts", At::End)?;
        let first = w.add_page(&drafts, "First draft", At::End)?;
        let second = w.add_page(&drafts, "Second draft", At::End)?;
        let old = w.add_section(&nb_id, "Old", At::End)?;
        w.add_page(&old, "Very old", At::End)?;
        let text = w.add_rich_text(
            &second,
            Frame::PAGE_TEXT,
            Some(&payload::rich_text_from_plain("Keep this")),
            &Settings::new(),
        )?;
        w.add_rich_text(
            &first,
            Frame::PAGE_TEXT,
            Some(&payload::rich_text_from_plain("Superseded text")),
            &Settings::new(),
        )?;
        let table = w.add_table_from_csv(
            &second,
            Frame::new(40, 400, 420, 180),
            TABLE_CSV.as_bytes(),
            &Settings::new(),
        )?;
        w.archive_node(&first, ArchiveReason::Superseded, None)?;
        w.archive_node(&old, ArchiveReason::Other, Some("kept for reference"))?;
        Ok((drafts, first, second, text, table))
    })?;
    nb.write(|w| {
        let archived_canvas = w.add_rich_text(
            &second,
            Frame::new(40, 600, 300, 60),
            Some(&payload::rich_text_from_plain("wrong figure")),
            &Settings::new(),
        )?;
        w.archive_canvas(&archived_canvas, ArchiveReason::Wrong, None)?;
        let back = w.add_rich_text(
            &second,
            Frame::new(400, 600, 200, 60),
            None,
            &Settings::new(),
        )?;
        w.archive_canvas(&back, ArchiveReason::Irrelevant, None)?;
        w.retrieve_canvas(&back)?;
        let (dataset, row): (String, String) = w.transaction().query_row(
            "SELECT d.id, r.id FROM datasets d JOIN dataset_rows r ON r.dataset_id = d.id \
             WHERE d.instance_id = ?1 ORDER BY r.seq LIMIT 1",
            [&table],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let cafe = w.tag("Café")?;
        w.add_tag_alias(&cafe, "Coffee")?;
        let project = w.tag("Work/Project Alpha")?;
        w.add_tag_alias(&project, "alpha")?;
        let urgent = w.tag("Urgent")?;
        let later = w.tag("Later")?;
        w.apply_tag(&project, "node", &second)?;
        w.apply_tag(&project, "node", &drafts)?;
        w.apply_tag(&cafe, "instance", &text)?;
        w.apply_tag(&urgent, "dataset", &dataset)?;
        w.apply_tag(&urgent, "dataset_row", &row)?;
        w.apply_tag(&later, "node", &second)?;
        w.apply_tag(&later, "node", &first)?;
        w.rename_tag(&urgent, "Urgent!")?;
        w.merge_tags(&later, &project)?;
        w.set_meta(
            "node",
            &second,
            "Vendor",
            &MetaValue::Text("Example Surveys".into()),
            "user",
        )?;
        w.set_meta(
            "node",
            &second,
            "vendor",
            &MetaValue::Text("Ëxample Two".into()),
            "user",
        )?;
        w.set_meta(
            "instance",
            &text,
            "capture_time",
            &MetaValue::Datetime {
                epoch_secs: 1_767_600_000.0,
                iso8601: "2026-01-05T08:00:00Z".into(),
            },
            "exif",
        )?;
        w.set_meta(
            "instance",
            &text,
            "geo",
            &MetaValue::Geo {
                lat: 51.5007,
                lon: -0.1246,
            },
            "user",
        )?;
        w.set_meta(
            "dataset_row",
            &row,
            "place",
            &MetaValue::Text("Westminster".into()),
            "user",
        )?;
        Ok(())
    })?;
    // Retrieved in a later session.
    nb.write(|w| w.retrieve_node(&first))
}

/// Calendars: the edge-case `.ics` (zoned, all-day, floating, attendees, attachments, skipped
/// events), a calendar used as a form field, and the page made into a template.
pub fn calendars(root: &Path) -> Result<()> {
    let mut nb = Notebook::create(root, Some("Written Calendars"))?;
    let nb_id = nb.notebook_node()?.id;
    nb.write(|w| {
        let section = w.add_section(&nb_id, "Diary", At::End)?;
        let page = w.add_page(&section, "January", At::End)?;
        let (cal, skipped) = w.add_calendar(
            &page,
            Frame::new(40, 40, 480, 360),
            CALENDAR_ICS.as_bytes(),
            &settings(json!({"scale": "week", "layoutMode": "standard"})),
        )?;
        assert_eq!(skipped, 2);
        let (form, _) = w.add_form(&section, "Booking", At::End)?;
        let (day, _) = w.add_calendar(
            &form,
            Frame::new(560, 40, 320, 240),
            CALENDAR_ICS.as_bytes(),
            &settings(json!({"displayDayEpoch": 1_767_571_200})),
        )?;
        w.set_form_field(&day, Some(&FormField::new("Day")))?;
        w.submit_form(
            &form,
            &Submission {
                submitted: Some("2026-01-05T09:30:00+00:00".into()),
                ..Submission::default()
            },
        )?;
        let _ = cal;
        w.make_template(&page)?;
        Ok(())
    })
}

/// An iCalendar file exercising what the import reads (and skips).
pub const CALENDAR_ICS: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//dunnenote-format//written//EN\r
BEGIN:VTIMEZONE\r
TZID:W. Europe Standard Time\r
END:VTIMEZONE\r
BEGIN:VEVENT\r
UID:zoned@example.invalid\r
SUMMARY:Planning (zoned)\r
LOCATION:Room 2\r
DESCRIPTION:Agenda\\nand notes\r
DTSTART;TZID=W. Europe Standard Time:20260105T090000\r
DURATION:PT1H30M\r
ORGANIZER;CN=Ada Example:mailto:ada@example.invalid\r
ATTENDEE;CN=Grace;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=TRUE:mailto:grace@example.invalid\r
ATTENDEE:mailto:bob@example.invalid\r
CATEGORIES:Work,Team\r
STATUS:CONFIRMED\r
URL:https://example.invalid/m\r
RRULE:FREQ=WEEKLY;COUNT=4\r
ATTACH;FMTTYPE=application/pdf;FILENAME=agenda.pdf:https://example.invalid/a.pdf\r
ATTACH;VALUE=BINARY;ENCODING=BASE64;X-FILENAME=inline.txt:aGVsbG8=\r
DTSTAMP:20260101T120000Z\r
LAST-MODIFIED:20260102T120000\r
SEQUENCE:3\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:backwards@example.invalid\r
DTSTART:20260105T100000Z\r
DTEND:20260105T090000Z\r
END:VEVENT\r
BEGIN:VEVENT\r
SUMMARY:Holiday\r
DTSTART;VALUE=DATE:20260107\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:utc@example.invalid\r
SUMMARY:Call\r
DTSTART:20260108T101500Z\r
DTEND:20260108T104500Z\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:floating@example.invalid\r
SUMMARY:Reminder\r
DTSTART:20260109T080000\r
END:VEVENT\r
BEGIN:VEVENT\r
SUMMARY:No start\r
END:VEVENT\r
END:VCALENDAR\r
";

pub const TABLE_CSV: &str = "Task;Owner;Hours;Done;Due;Blank\n\
    Survey;Ada;3;TRUE;2026-01-05;\n\
    Report;;5.5;false;2026-01-06T09:30;\n\
    \"Quoted; text\";Grace;-2;true;not a date;\n";

pub const TABLE_JSON: &str = r#"[{"Item":"Tent","Qty":2,"Packed":true,"Meta":{"colour":"green"}},
{"Item":"Stove","Qty":1.5,"Packed":null},{"Item":"Rope"}]"#;

pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".dunnenote.lock" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Build every written notebook under `out`, returning `(name, path)` pairs.
pub fn build_all(out: &Path) -> Result<Vec<(&'static str, PathBuf)>> {
    let mut built = Vec::new();
    for (name, build) in [
        ("written-minimal", minimal as fn(&Path) -> Result<()>),
        ("written-every-kind", every_kind),
        ("written-tables", tables),
        ("written-forms", forms),
        ("written-templates", templates),
        ("written-archive-tags", archive_tags),
        ("written-calendars", calendars),
    ] {
        let root = out.join(format!("{name}.dunnenote"));
        build(&root)?;
        built.push((name, root));
    }
    let root = out.join("written-into-golden.dunnenote");
    into_golden(&root, &fixtures().join("every-kind.dunnenote"))?;
    built.push(("written-into-golden", root));
    Ok(built)
}
