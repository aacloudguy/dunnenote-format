//! Calendars: importing DunneNote's golden `.ics` again must give its event and attendee rows,
//! column for column.

use std::path::Path;

use dunnenote_format::{CanvasKind, Error, Frame, Notebook, Settings};
use rusqlite::types::Value as Sql;
use tempfile::TempDir;

#[path = "support/common.rs"]
mod common;
use common::{assert_clean, count, new_notebook, page_in, raw};

#[path = "support/written.rs"]
mod written;
use written::fixtures;

const EVENT_COLUMNS: &str = "uid, summary, location, description, start_utc, end_utc, all_day, \
    tzid, source_ordinal, url, organizer_value, organizer_cn, status, categories, rrule_text, \
    attachments, dtstamp_utc, last_modified_utc, sequence_no";

/// Every stored column of a calendar's events (ids and timestamps aside), with its attendees.
fn rows(root: &Path, canvas: &str) -> Vec<(Vec<Sql>, Vec<Vec<Sql>>)> {
    let conn = raw(root);
    let n = EVENT_COLUMNS.split(',').count();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, {EVENT_COLUMNS} FROM calendar_events WHERE instance_id = ?1 \
             ORDER BY source_ordinal"
        ))
        .unwrap();
    let events: Vec<(String, Vec<Sql>)> = stmt
        .query_map([canvas], |r| {
            Ok((r.get(0)?, (1..=n).map(|i| r.get(i).unwrap()).collect()))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    events
        .into_iter()
        .map(|(id, cols)| {
            let attendees = conn
                .prepare(
                    "SELECT ordinal, value, cn, role, partstat, rsvp FROM calendar_event_attendees \
                     WHERE event_id = ?1 ORDER BY ordinal",
                )
                .unwrap()
                .query_map([&id], |r| Ok((0..6).map(|i| r.get(i).unwrap()).collect()))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            (cols, attendees)
        })
        .collect()
}

#[test]
fn an_ics_import_matches_dunnenotes() {
    let golden_root = fixtures().join("every-kind.dunnenote");
    let golden = Notebook::open(&golden_root).unwrap();
    let gcal = golden
        .pages()
        .unwrap()
        .iter()
        .flat_map(|p| golden.canvases(&p.id).unwrap())
        .find(|c| c.kind == CanvasKind::Calendar)
        .unwrap();
    let ics = golden.read_blob(&gcal.source_hash).unwrap();

    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Calendar");
    let page = page_in(&mut nb, &root);
    let (cal, skipped) = nb
        .write(|w| w.add_calendar(&page, Frame::new(40, 40, 480, 360), &ics, &Settings::new()))
        .unwrap();
    assert_eq!(skipped, 0);
    let c = nb.canvas(&cal).unwrap();
    assert_eq!(
        (c.kind, c.source_hash.as_str()),
        (CanvasKind::Calendar, gcal.source_hash.as_str())
    );
    assert_eq!(
        Sql::Text(c.settings.len().to_string()),
        Sql::Text("0".into())
    );

    let ours = rows(nb.root(), &cal);
    let theirs = rows(&golden_root, &gcal.id);
    assert!(!theirs.is_empty());
    assert!(
        theirs.iter().any(|(_, a)| !a.is_empty()),
        "the golden calendar has attendees"
    );
    assert_eq!(ours, theirs);
    assert_clean(nb.root());
}

#[test]
fn a_bad_file_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut nb, root) = new_notebook(&dir, "Bad");
    let page = page_in(&mut nb, &root);
    let path = nb.root().to_path_buf();
    let blobs = count(&path, "SELECT count(*) FROM blobs");
    for bad in [&b""[..], b"hello", b"BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n"] {
        let r =
            nb.write(|w| w.add_calendar(&page, Frame::new(0, 0, 100, 100), bad, &Settings::new()));
        assert!(matches!(r, Err(Error::Invalid(_))), "{r:?}");
    }
    assert_eq!(
        count(&path, "SELECT count(*) FROM blobs"),
        blobs,
        "no blob stored for a refused file"
    );
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM canvas_instances WHERE kind = 'calendar'"
        ),
        0
    );
}
