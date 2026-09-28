//! Calendar canvases: an `.ics` file shown on a page, with its events stored as rows.

use rusqlite::params;

use super::{new_id, Frame, Writer};
use crate::error::Result;
use crate::ics::{parse_ics, IcsCalendar};
use crate::model::{CanvasKind, Settings};

/// The frame DunneNote gives a new calendar.
pub const CALENDAR_SIZE: (i64, i64) = (480, 360);

impl Writer<'_> {
    /// Add a Calendar canvas showing `ics` (an iCalendar file), as DunneNote's import does: the
    /// file is parsed first, stored as the canvas's source blob only if it parses, and every
    /// readable event (with its attendees) stored in the same transaction. Returns the canvas
    /// and the number of events skipped.
    pub fn add_calendar(
        &mut self,
        page: &str,
        frame: Frame,
        ics: &[u8],
        settings: &Settings,
    ) -> Result<(String, usize)> {
        let calendar = parse_ics(ics)?;
        let hash = self.put_blob(ics)?;
        let id = self.insert_canvas(page, CanvasKind::Calendar, &hash, frame, settings)?;
        self.insert_events(&id, &calendar)?;
        Ok((id, calendar.skipped))
    }

    fn insert_events(&mut self, canvas: &str, calendar: &IcsCalendar) -> Result<()> {
        let json_list = |items: Vec<serde_json::Value>| -> Option<String> {
            (!items.is_empty()).then(|| serde_json::Value::Array(items).to_string())
        };
        for ev in &calendar.events {
            let id = new_id();
            let categories = json_list(ev.categories.iter().map(|c| c.clone().into()).collect());
            let attachments = json_list(
                ev.attachments
                    .iter()
                    .map(|a| serde_json::to_value(a).expect("plain struct"))
                    .collect(),
            );
            self.tx.execute(
                "INSERT INTO calendar_events (id, instance_id, uid, summary, location, description, \
                   start_utc, end_utc, all_day, tzid, created_at, updated_at, source_ordinal, url, \
                   organizer_value, organizer_cn, status, categories, rrule_text, attachments, \
                   dtstamp_utc, last_modified_utc, sequence_no) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, unixepoch(), unixepoch(), \
                   ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
                params![
                    id,
                    canvas,
                    ev.uid,
                    ev.summary,
                    ev.location,
                    ev.description,
                    ev.start_utc,
                    ev.end_utc,
                    i64::from(ev.all_day),
                    ev.tzid,
                    ev.source_ordinal,
                    ev.url,
                    ev.organizer_value,
                    ev.organizer_cn,
                    ev.status,
                    categories,
                    ev.rrule_text,
                    attachments,
                    ev.dtstamp_utc,
                    ev.last_modified_utc,
                    ev.sequence,
                ],
            )?;
            for (ordinal, a) in ev.attendees.iter().enumerate() {
                self.tx.execute(
                    "INSERT INTO calendar_event_attendees (id, event_id, ordinal, value, cn, role, \
                     partstat, rsvp, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, unixepoch(), unixepoch())",
                    params![
                        new_id(),
                        id,
                        ordinal as i64,
                        a.value,
                        a.cn,
                        a.role,
                        a.partstat,
                        i64::from(a.rsvp)
                    ],
                )?;
            }
        }
        Ok(())
    }
}
