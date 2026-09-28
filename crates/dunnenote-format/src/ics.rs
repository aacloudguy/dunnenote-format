//! Reading an iCalendar (`.ics`) file into the event rows a Calendar canvas stores, as DunneNote
//! imports it (`SPEC.md` section 8, "Calendars").
//!
//! Every `VEVENT` is one row; other components are ignored. Times are anchored without applying
//! any time zone: a UTC time is its instant, a floating or `TZID=` time is its wall-clock time read
//! as if it were UTC (the `TZID` is kept), and a date is 00:00 UTC of that day, all day.
//! Recurrences are not expanded (`RRULE` is kept as text). An event whose times cannot be read is
//! skipped, not an error.

use icalendar::{CalendarComponent, CalendarDateTime, Component, DatePerhapsTime, EventLike};
use serde::Serialize;

use crate::error::{Error, Result};

/// Largest `.ics` file DunneNote imports.
pub const MAX_ICS_BYTES: usize = 5 * 1024 * 1024;
/// Most events one calendar keeps; the rest are skipped.
pub const MAX_EVENTS: usize = 10_000;

const DAY: i64 = 86_400;

// Character limits.
const MAX_SUMMARY: usize = 1_024;
const MAX_LOCATION: usize = 1_024;
const MAX_DESCRIPTION: usize = 16_384;
const MAX_UID: usize = 512;
const MAX_TZID: usize = 128;
const MAX_URL: usize = 2_048;
const MAX_ORGANIZER: usize = 512;
const MAX_CN: usize = 512;
const MAX_STATUS: usize = 64;
const MAX_CATEGORY: usize = 256;
const MAX_CATEGORIES: usize = 64;
const MAX_RRULE: usize = 1_024;
const MAX_ATTENDEES: usize = 512;
const MAX_ATTENDEE_VALUE: usize = 512;
const MAX_PARAM: usize = 64;
const MAX_ATTACHMENTS: usize = 64;
const MAX_ATTACHMENT_FILENAME: usize = 512;
const MAX_ATTACHMENT_FMTTYPE: usize = 128;
const MAX_ATTACHMENT_URI: usize = 2_048;

/// One attendee row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcsAttendee {
    pub value: String,
    pub cn: Option<String>,
    pub role: Option<String>,
    pub partstat: Option<String>,
    pub rsvp: bool,
}

/// One attachment, as stored in `calendar_events.attachments` (fields in this order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IcsAttachment {
    pub filename: Option<String>,
    pub fmttype: Option<String>,
    pub uri: Option<String>,
}

/// One event row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcsEvent {
    pub uid: Option<String>,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub all_day: bool,
    pub tzid: Option<String>,
    /// Position among the file's `VEVENT`s, counting skipped ones.
    pub source_ordinal: i64,
    pub url: Option<String>,
    pub organizer_value: Option<String>,
    pub organizer_cn: Option<String>,
    pub status: Option<String>,
    pub categories: Vec<String>,
    pub rrule_text: Option<String>,
    pub attachments: Vec<IcsAttachment>,
    pub attendees: Vec<IcsAttendee>,
    pub dtstamp_utc: Option<i64>,
    pub last_modified_utc: Option<i64>,
    pub sequence: Option<i64>,
}

/// A parsed calendar file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcsCalendar {
    pub events: Vec<IcsEvent>,
    /// Events left out: unreadable times, an end before the start, or past [`MAX_EVENTS`].
    pub skipped: usize,
}

/// Whether the file starts, after an optional UTF-8 byte-order mark and whitespace, with
/// `BEGIN:VCALENDAR` (any case) within its first 4 KiB.
fn looks_like_vcalendar(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    let start = head
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(head.len());
    let token = b"BEGIN:VCALENDAR";
    head.len() - start >= token.len()
        && head[start..start + token.len()].eq_ignore_ascii_case(token)
}

/// Parse an `.ics` file. Refuses an empty file, one over [`MAX_ICS_BYTES`], and one that is not
/// an iCalendar document.
pub fn parse_ics(bytes: &[u8]) -> Result<IcsCalendar> {
    if bytes.is_empty() {
        return Err(Error::Invalid("the calendar file is empty".into()));
    }
    if bytes.len() > MAX_ICS_BYTES {
        return Err(Error::Invalid(format!(
            "the calendar file is {} bytes; DunneNote imports at most {MAX_ICS_BYTES}",
            bytes.len()
        )));
    }
    if !looks_like_vcalendar(bytes) {
        return Err(Error::Invalid(
            "not an iCalendar file (it does not start with BEGIN:VCALENDAR)".into(),
        ));
    }
    let text = String::from_utf8_lossy(bytes);
    let calendar: icalendar::Calendar = text
        .parse()
        .map_err(|_| Error::Invalid("the calendar file could not be read".into()))?;
    let mut events = Vec::new();
    let mut skipped = 0;
    let mut ordinal: i64 = 0;
    for component in calendar.iter() {
        let CalendarComponent::Event(ev) = component else {
            continue;
        };
        let this = ordinal;
        ordinal = ordinal.saturating_add(1);
        match build_event(ev, this) {
            Some(e) if events.len() < MAX_EVENTS => events.push(e),
            _ => skipped += 1,
        }
    }
    Ok(IcsCalendar { events, skipped })
}

fn build_event(ev: &icalendar::Event, source_ordinal: i64) -> Option<IcsEvent> {
    let (start_utc, all_day, tzid) = anchor(&ev.get_start()?)?;
    let end_utc = if let Some(end) = ev.get_end() {
        anchor(&end)?.0
    } else if let Some(d) = ev.property_value("DURATION").and_then(parse_duration) {
        start_utc.checked_add(d)?
    } else if all_day {
        start_utc.checked_add(DAY)?
    } else {
        start_utc
    };
    if end_utc < start_utc {
        return None;
    }
    let (organizer_value, organizer_cn) = match ev.properties().get("ORGANIZER") {
        None => (None, None),
        Some(p) => {
            let v = p.value().trim();
            (
                (!v.is_empty()).then(|| cut(v, MAX_ORGANIZER)),
                param(p, "CN", MAX_CN),
            )
        }
    };
    Some(IcsEvent {
        uid: ev.get_uid().map(|s| cut(s, MAX_UID)),
        summary: cut(ev.get_summary().unwrap_or(""), MAX_SUMMARY),
        location: cut(ev.get_location().unwrap_or(""), MAX_LOCATION),
        description: cut(ev.get_description().unwrap_or(""), MAX_DESCRIPTION),
        start_utc,
        end_utc,
        all_day,
        tzid: tzid.map(|t| cut(&t, MAX_TZID)),
        source_ordinal,
        url: single(ev, "URL", MAX_URL),
        organizer_value,
        organizer_cn,
        status: single(ev, "STATUS", MAX_STATUS),
        categories: categories(ev),
        rrule_text: single(ev, "RRULE", MAX_RRULE),
        attachments: attachments(ev),
        attendees: attendees(ev),
        dtstamp_utc: stamp(ev, "DTSTAMP"),
        last_modified_utc: stamp(ev, "LAST-MODIFIED"),
        sequence: ev
            .property_value("SEQUENCE")
            .and_then(|v| v.trim().parse().ok()),
    })
}

fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn single(ev: &icalendar::Event, key: &str, max: usize) -> Option<String> {
    let v = ev.property_value(key)?.trim();
    (!v.is_empty()).then(|| cut(v, max))
}

fn param(p: &icalendar::Property, key: &str, max: usize) -> Option<String> {
    let v = p.get_param_as(key, |s| Some(s.to_string()))?;
    let v = v.trim();
    (!v.is_empty()).then(|| cut(v, max))
}

fn categories(ev: &icalendar::Event) -> Vec<String> {
    let mut out = Vec::new();
    for p in ev
        .multi_properties()
        .get("CATEGORIES")
        .into_iter()
        .flatten()
    {
        for part in p.value().split(',') {
            if out.len() >= MAX_CATEGORIES {
                return out;
            }
            let part = part.trim();
            if !part.is_empty() {
                out.push(cut(part, MAX_CATEGORY));
            }
        }
    }
    out
}

fn attendees(ev: &icalendar::Event) -> Vec<IcsAttendee> {
    ev.multi_properties()
        .get("ATTENDEE")
        .into_iter()
        .flatten()
        .take(MAX_ATTENDEES)
        .map(|p| IcsAttendee {
            value: cut(p.value().trim(), MAX_ATTENDEE_VALUE),
            cn: param(p, "CN", MAX_CN),
            role: param(p, "ROLE", MAX_PARAM),
            partstat: param(p, "PARTSTAT", MAX_PARAM),
            rsvp: param(p, "RSVP", MAX_PARAM).is_some_and(|v| v.eq_ignore_ascii_case("TRUE")),
        })
        .collect()
}

fn attachments(ev: &icalendar::Event) -> Vec<IcsAttachment> {
    ev.multi_properties()
        .get("ATTACH")
        .into_iter()
        .flatten()
        .take(MAX_ATTACHMENTS)
        .map(|p| {
            let inline = param(p, "VALUE", MAX_PARAM)
                .is_some_and(|v| v.eq_ignore_ascii_case("BINARY"))
                || param(p, "ENCODING", MAX_PARAM)
                    .is_some_and(|v| v.eq_ignore_ascii_case("BASE64"));
            let uri = if inline {
                None
            } else {
                let v = p.value().trim();
                (!v.is_empty()).then(|| cut(v, MAX_ATTACHMENT_URI))
            };
            IcsAttachment {
                filename: param(p, "FILENAME", MAX_ATTACHMENT_FILENAME)
                    .or_else(|| param(p, "X-FILENAME", MAX_ATTACHMENT_FILENAME)),
                fmttype: param(p, "FMTTYPE", MAX_ATTACHMENT_FMTTYPE),
                uri,
            }
        })
        .collect()
}

fn stamp(ev: &icalendar::Event, key: &str) -> Option<i64> {
    Some(
        match ev
            .property_value(key)?
            .trim()
            .parse::<CalendarDateTime>()
            .ok()?
        {
            CalendarDateTime::Utc(dt) => dt.timestamp(),
            CalendarDateTime::Floating(ndt) => ndt.and_utc().timestamp(),
            CalendarDateTime::WithTimezone { date_time, .. } => date_time.and_utc().timestamp(),
        },
    )
}

/// `(seconds, all day, tzid)` of a start or end, without applying any zone.
fn anchor(value: &DatePerhapsTime) -> Option<(i64, bool, Option<String>)> {
    Some(match value {
        DatePerhapsTime::Date(d) => (d.and_hms_opt(0, 0, 0)?.and_utc().timestamp(), true, None),
        DatePerhapsTime::DateTime(CalendarDateTime::Utc(dt)) => {
            (dt.timestamp(), false, Some("UTC".into()))
        }
        DatePerhapsTime::DateTime(CalendarDateTime::Floating(ndt)) => {
            (ndt.and_utc().timestamp(), false, None)
        }
        DatePerhapsTime::DateTime(CalendarDateTime::WithTimezone { date_time, tzid }) => {
            (date_time.and_utc().timestamp(), false, Some(tzid.clone()))
        }
    })
}

/// An RFC 5545 duration (`[+-]P…W` or `[+-]P[nD][T[nH][nM][nS]]`) in seconds.
fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1i64, r),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let rest = rest.strip_prefix('P')?;
    if rest.is_empty() {
        return None;
    }
    if let Some(weeks) = rest.strip_suffix('W') {
        return weeks
            .parse::<i64>()
            .ok()?
            .checked_mul(7 * DAY)?
            .checked_mul(sign);
    }
    let (date, time) = rest.split_once('T').unwrap_or((rest, ""));
    let mut total: i64 = 0;
    if !date.is_empty() {
        total = date
            .strip_suffix('D')?
            .parse::<i64>()
            .ok()?
            .checked_mul(DAY)?;
    }
    let mut num = String::new();
    for ch in time.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        let n: i64 = num.parse().ok()?;
        num.clear();
        total = total.checked_add(match ch {
            'H' => n.checked_mul(3600)?,
            'M' => n.checked_mul(60)?,
            'S' => n,
            _ => return None,
        })?;
    }
    if !num.is_empty() {
        return None;
    }
    total.checked_mul(sign)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGES: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//test//EN\r
BEGIN:VTODO\r
UID:todo\r
END:VTODO\r
BEGIN:VEVENT\r
UID:tz\r
SUMMARY:Zoned\r
DTSTART;TZID=W. Europe Standard Time:20260105T090000\r
DURATION:PT1H30M\r
ATTENDEE;CN=Ada;ROLE=CHAIR;PARTSTAT=ACCEPTED;RSVP=true:mailto:ada@example.com\r
ATTENDEE;RSVP=FALSE:mailto:bob@example.com\r
ORGANIZER;CN=\"Grace H\":mailto:grace@example.com\r
CATEGORIES:Work, Team\r
CATEGORIES:,Extra\r
STATUS: CONFIRMED \r
URL:https://example.com/m\r
RRULE:FREQ=WEEKLY;COUNT=4\r
ATTACH;FMTTYPE=application/pdf;FILENAME=agenda.pdf:https://example.com/a.pdf\r
ATTACH;VALUE=BINARY;ENCODING=BASE64;X-FILENAME=inline.txt:aGVsbG8=\r
ATTACH;ENCODING=BASE64;FMTTYPE=text/plain:aGk=\r
DTSTAMP:20260101T120000Z\r
LAST-MODIFIED:20260102T120000\r
SEQUENCE: 3\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:bad\r
DTSTART:20260105T100000Z\r
DTEND:20260105T090000Z\r
END:VEVENT\r
BEGIN:VEVENT\r
SUMMARY:All day\r
DTSTART;VALUE=DATE:20260107\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:point\r
DTSTART:20260108T101500\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:nostart\r
SUMMARY:No start\r
END:VEVENT\r
END:VCALENDAR\r
";

    #[test]
    fn events_are_read_as_dunnenote_reads_them() {
        let cal = parse_ics(EDGES.as_bytes()).unwrap();
        assert_eq!(cal.skipped, 2, "an end before its start, and no start");
        let [zoned, all_day, point] = &cal.events[..] else {
            panic!("{:?}", cal.events)
        };
        assert_eq!(zoned.source_ordinal, 0);
        assert_eq!(zoned.tzid.as_deref(), Some("W. Europe Standard Time"));
        assert_eq!(zoned.start_utc, 1_767_603_600, "wall clock read as UTC");
        assert_eq!(zoned.end_utc - zoned.start_utc, 5400);
        assert_eq!(zoned.attendees.len(), 2);
        assert_eq!(
            zoned.attendees[0],
            IcsAttendee {
                value: "mailto:ada@example.com".into(),
                cn: Some("Ada".into()),
                role: Some("CHAIR".into()),
                partstat: Some("ACCEPTED".into()),
                rsvp: true
            }
        );
        assert!(!zoned.attendees[1].rsvp);
        assert_eq!(zoned.organizer_cn.as_deref(), Some("Grace H"));
        assert_eq!(zoned.categories, ["Work", "Team", "Extra"]);
        assert_eq!(zoned.status.as_deref(), Some("CONFIRMED"));
        assert_eq!(zoned.rrule_text.as_deref(), Some("FREQ=WEEKLY;COUNT=4"));
        assert_eq!(
            serde_json::to_string(&zoned.attachments).unwrap(),
            r#"[{"filename":"agenda.pdf","fmttype":"application/pdf","uri":"https://example.com/a.pdf"},{"filename":"inline.txt","fmttype":null,"uri":null},{"filename":null,"fmttype":"text/plain","uri":null}]"#
        );
        assert_eq!(zoned.dtstamp_utc, Some(1_767_268_800));
        assert_eq!(zoned.last_modified_utc, Some(1_767_355_200));
        assert_eq!(zoned.sequence, Some(3));

        assert_eq!(all_day.source_ordinal, 2, "ordinals count skipped events");
        assert!(all_day.all_day && all_day.tzid.is_none() && all_day.uid.is_none());
        assert_eq!(all_day.end_utc - all_day.start_utc, 86_400);
        assert_eq!(point.start_utc, point.end_utc);
        assert_eq!(point.tzid, None, "floating");
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("P1W"), Some(604_800));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("P1DT2H3M4S"), Some(93_784));
        assert_eq!(parse_duration("PT"), Some(0), "as DunneNote reads it");
        for bad in ["P", "1D", "PT5", "P1H", "PX"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn refusals() {
        assert!(parse_ics(b"").is_err());
        assert!(parse_ics(b"hello\r\nBEGIN:VCALENDAR").is_err());
        let mut late = vec![b' '; 4100];
        late.extend_from_slice(b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n");
        assert!(parse_ics(&late).is_err());
        assert!(parse_ics(b"begin:vcalendar\r\nend:vcalendar\r\n").is_ok());
        // Leading whitespace or a byte-order mark passes the first check, but the iCalendar
        // parser (the same one DunneNote uses) cannot read the file, so it is refused, as
        // DunneNote refuses it.
        assert!(parse_ics(b"\r\n  BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n").is_err());
        assert!(parse_ics(b"\xEF\xBB\xBFBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n").is_err());
    }
}
