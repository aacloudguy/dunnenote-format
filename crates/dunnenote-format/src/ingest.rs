//! Turning CSV and JSON files into table columns and rows, as DunneNote imports them.
//!
//! The rules (`SPEC.md` section 8):
//!
//! - A UTF-8 byte-order mark is dropped. Input that is empty or only whitespace is refused.
//! - **CSV:** the delimiter is whichever of `,` `;` and tab appears most often in the first 8 KiB
//!   (a comma on a tie). The first record is the header row. Fields are trimmed, rows may have
//!   different lengths, and the widest row sets the column count.
//! - **JSON:** a top-level array of objects gives one column per key (in order of first
//!   appearance); an array of arrays gives positional columns "Column 1", …; an array of scalars
//!   gives one column "value"; a single object gives "Key" / "Value" rows. When the file is not
//!   one JSON document it is read as JSON Lines. Nested objects and arrays are stored as their
//!   compact JSON text; `null` is left out.
//! - Columns get keys `c0`, `c1`, … in order and are named after the header (a blank header is
//!   "Column N"). Each column's type hint is `number`, `boolean` or `date` when every value
//!   present has that type (for CSV, a number is anything that parses as an integer or finite
//!   float; a boolean is `true`/`false` in any case; a date starts `YYYY-MM-DD`), `unknown` when
//!   the column has no values, and `text` otherwise. CSV numbers and booleans are stored as JSON
//!   numbers and booleans; everything else is a string.
//! - Empty values are left out of a row's `cells`.
//! - At most [`MAX_COLUMNS`] columns (more is refused) and [`MAX_ROWS`] rows (the rest are
//!   dropped and counted); a string longer than [`MAX_CELL_BYTES`] is cut at a character
//!   boundary and counted.

use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// Most columns a table may have.
pub const MAX_COLUMNS: usize = 256;
/// Most rows an import keeps.
pub const MAX_ROWS: usize = 50_000;
/// Longest text cell, in bytes.
pub const MAX_CELL_BYTES: usize = 64 * 1024;
/// Largest file DunneNote imports as a table.
pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;

/// A column's type hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeHint {
    Text,
    Number,
    Date,
    Boolean,
    Unknown,
}

impl TypeHint {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "number" => Some(Self::Number),
            "date" => Some(Self::Date),
            "boolean" => Some(Self::Boolean),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Date => "date",
            Self::Boolean => "boolean",
            Self::Unknown => "unknown",
        }
    }
}

/// A column to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewColumn {
    pub col_key: String,
    pub name: String,
    pub type_hint: TypeHint,
    pub position: i64,
}

/// A parsed table, ready to store.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTable {
    pub columns: Vec<NewColumn>,
    pub rows: Vec<Map<String, Value>>,
    /// Rows dropped past [`MAX_ROWS`].
    pub rows_truncated: usize,
    /// Cells cut to [`MAX_CELL_BYTES`].
    pub cells_truncated: usize,
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn check_input(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::Invalid(format!(
            "the file is {} bytes; DunneNote imports at most {MAX_FILE_BYTES}",
            bytes.len()
        )));
    }
    if strip_bom(bytes).iter().all(u8::is_ascii_whitespace) {
        return Err(Error::Invalid("the file is empty".into()));
    }
    Ok(())
}

fn cap_string(raw: &str, truncated: &mut usize) -> Value {
    if raw.len() <= MAX_CELL_BYTES {
        return Value::String(raw.to_string());
    }
    let mut end = MAX_CELL_BYTES;
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    *truncated += 1;
    Value::String(raw[..end].to_string())
}

/// Whether `s` starts with `YYYY-MM-DD`, alone or followed by `T` or a space.
pub(crate) fn looks_like_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 10 {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && (b.len() == 10 || matches!(b[10], b'T' | b' '))
}

fn parse_number(s: &str) -> Option<Value> {
    if let Ok(i) = s.parse::<i64>() {
        return Some(Value::from(i));
    }
    match s.parse::<f64>() {
        Ok(f) if f.is_finite() => serde_json::Number::from_f64(f).map(Value::Number),
        _ => None,
    }
}

/// The value DunneNote stores when text is typed into an Editable table cell: a number when the
/// trimmed text is `-?digits(.digits)?`, `""` when it is blank, and otherwise the text as typed.
pub fn cell_from_text(text: &str) -> Value {
    let t = text.trim();
    if t.is_empty() {
        return Value::String(String::new());
    }
    let numeric = {
        let digits = t.strip_prefix('-').unwrap_or(t);
        let (int, frac) = match digits.split_once('.') {
            Some((i, f)) => (i, Some(f)),
            None => (digits, None),
        };
        let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        all_digits(int) && frac.is_none_or(all_digits)
    };
    if numeric {
        if let Ok(n) = t.parse::<f64>() {
            // As JavaScript's JSON.stringify writes a number: integers without a fraction.
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
                return Value::from(n as i64);
            }
            if let Some(num) = serde_json::Number::from_f64(n) {
                return Value::Number(num);
            }
        }
    }
    Value::String(text.to_string())
}

fn is_bool_text(s: &str) -> bool {
    s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false")
}

fn column(position: usize, name: String, type_hint: TypeHint) -> NewColumn {
    NewColumn {
        col_key: format!("c{position}"),
        name,
        type_hint,
        position: position as i64,
    }
}

fn detect_delimiter(bytes: &[u8]) -> u8 {
    let window = &bytes[..bytes.len().min(8 * 1024)];
    let count = |d: u8| window.iter().filter(|&&b| b == d).count();
    let mut best = (b',', count(b','));
    for d in [b';', b'\t'] {
        let n = count(d);
        if n > best.1 {
            best = (d, n);
        }
    }
    best.0
}

/// Parse a CSV file whose first record is the header.
pub fn parse_csv(bytes: &[u8]) -> Result<ParsedTable> {
    check_input(bytes)?;
    let bytes = strip_bom(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(detect_delimiter(bytes))
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(bytes);

    let mut header: Option<Vec<String>> = None;
    let mut data: Vec<Vec<String>> = Vec::new();
    let mut n_cols = 0;
    let mut rows_truncated = 0;
    let mut record = csv::ByteRecord::new();
    while reader
        .read_byte_record(&mut record)
        .map_err(|e| Error::Invalid(format!("not readable as CSV: {e}")))?
    {
        if record.len() > MAX_COLUMNS {
            return Err(too_many_columns());
        }
        let fields: Vec<String> = record
            .iter()
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect();
        n_cols = n_cols.max(fields.len());
        if header.is_none() {
            header = Some(fields);
        } else if data.len() >= MAX_ROWS {
            rows_truncated += 1;
        } else {
            data.push(fields);
        }
    }
    if n_cols == 0 {
        return Err(Error::Invalid("the file has no columns".into()));
    }

    let hints: Vec<TypeHint> = (0..n_cols)
        .map(|i| {
            let values: Vec<&str> = data
                .iter()
                .filter_map(|r| r.get(i))
                .map(String::as_str)
                .filter(|v| !v.is_empty())
                .collect();
            if values.is_empty() {
                TypeHint::Unknown
            } else if values.iter().all(|v| parse_number(v).is_some()) {
                TypeHint::Number
            } else if values.iter().all(|v| is_bool_text(v)) {
                TypeHint::Boolean
            } else if values.iter().all(|v| looks_like_iso_date(v)) {
                TypeHint::Date
            } else {
                TypeHint::Text
            }
        })
        .collect();
    let columns = (0..n_cols)
        .map(|i| {
            let name = match header.as_ref().and_then(|h| h.get(i)) {
                Some(h) if !h.is_empty() => h.clone(),
                _ => format!("Column {}", i + 1),
            };
            column(i, name, hints[i])
        })
        .collect();

    let mut cells_truncated = 0;
    let rows = data
        .iter()
        .map(|row| {
            let mut cells = Map::new();
            for (i, hint) in hints.iter().enumerate() {
                let Some(raw) = row.get(i).filter(|v| !v.is_empty()) else {
                    continue;
                };
                let value = match hint {
                    TypeHint::Number => parse_number(raw),
                    TypeHint::Boolean if is_bool_text(raw) => {
                        Some(Value::Bool(raw.eq_ignore_ascii_case("true")))
                    }
                    _ => None,
                }
                .unwrap_or_else(|| cap_string(raw, &mut cells_truncated));
                cells.insert(format!("c{i}"), value);
            }
            cells
        })
        .collect();
    Ok(ParsedTable {
        columns,
        rows,
        rows_truncated,
        cells_truncated,
    })
}

fn too_many_columns() -> Error {
    Error::Invalid(format!("a table has at most {MAX_COLUMNS} columns"))
}

/// Parse a JSON (or JSON Lines) file.
pub fn parse_json(bytes: &[u8]) -> Result<ParsedTable> {
    check_input(bytes)?;
    let bytes = strip_bom(bytes);
    let items = match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Array(items)) => items,
        Ok(Value::Object(map)) => return single_object(&map),
        Ok(scalar) => vec![scalar],
        Err(_) => {
            let text = String::from_utf8_lossy(bytes);
            let mut values = Vec::new();
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                values.push(
                    serde_json::from_str::<Value>(line)
                        .map_err(|e| Error::Invalid(format!("not readable as JSON: {e}")))?,
                );
            }
            if values.is_empty() {
                return Err(Error::Invalid("not readable as JSON".into()));
            }
            values
        }
    };

    let rows_truncated = items.len().saturating_sub(MAX_ROWS);
    let items = &items[..items.len().min(MAX_ROWS)];
    let mut cells_truncated = 0;
    let mut cell = |v: &Value| -> Option<Value> {
        match v {
            Value::Null => None,
            Value::Bool(_) | Value::Number(_) => Some(v.clone()),
            Value::String(s) => Some(cap_string(s, &mut cells_truncated)),
            other => Some(cap_string(&other.to_string(), &mut cells_truncated)),
        }
    };

    let (names, rows): (Vec<String>, Vec<Map<String, Value>>) =
        if items.iter().any(Value::is_object) {
            let mut names: Vec<String> = Vec::new();
            for obj in items.iter().filter_map(Value::as_object) {
                for key in obj.keys() {
                    if !names.contains(key) {
                        names.push(key.clone());
                        if names.len() > MAX_COLUMNS {
                            return Err(too_many_columns());
                        }
                    }
                }
            }
            let rows = items
                .iter()
                .map(|item| {
                    let mut cells = Map::new();
                    for (k, v) in item.as_object().into_iter().flatten() {
                        let idx = names.iter().position(|n| n == k).expect("collected above");
                        if let Some(value) = cell(v) {
                            cells.insert(format!("c{idx}"), value);
                        }
                    }
                    cells
                })
                .collect();
            (names, rows)
        } else if items.iter().any(Value::is_array) {
            let n = items
                .iter()
                .filter_map(Value::as_array)
                .map(Vec::len)
                .max()
                .unwrap_or(0);
            if n > MAX_COLUMNS {
                return Err(too_many_columns());
            }
            let rows = items
                .iter()
                .map(|item| {
                    let mut cells = Map::new();
                    for (i, v) in item.as_array().into_iter().flatten().enumerate() {
                        if let Some(value) = cell(v) {
                            cells.insert(format!("c{i}"), value);
                        }
                    }
                    cells
                })
                .collect();
            ((0..n).map(|i| format!("Column {}", i + 1)).collect(), rows)
        } else {
            let rows = items
                .iter()
                .map(|v| {
                    let mut cells = Map::new();
                    if let Some(value) = cell(v) {
                        cells.insert("c0".into(), value);
                    }
                    cells
                })
                .collect();
            (vec!["value".into()], rows)
        };
    finish(names, rows, rows_truncated, cells_truncated)
}

fn single_object(map: &Map<String, Value>) -> Result<ParsedTable> {
    let mut cells_truncated = 0;
    let rows: Vec<Map<String, Value>> = map
        .iter()
        .take(MAX_ROWS)
        .map(|(k, v)| {
            let mut cells = Map::new();
            cells.insert("c0".into(), Value::String(k.clone()));
            let value = match v {
                Value::Null => None,
                Value::Bool(_) | Value::Number(_) => Some(v.clone()),
                Value::String(s) => Some(cap_string(s, &mut cells_truncated)),
                other => Some(cap_string(&other.to_string(), &mut cells_truncated)),
            };
            if let Some(value) = value {
                cells.insert("c1".into(), value);
            }
            cells
        })
        .collect();
    let rows_truncated = map.len().saturating_sub(rows.len());
    finish(
        vec!["Key".into(), "Value".into()],
        rows,
        rows_truncated,
        cells_truncated,
    )
}

fn finish(
    names: Vec<String>,
    rows: Vec<Map<String, Value>>,
    rows_truncated: usize,
    cells_truncated: usize,
) -> Result<ParsedTable> {
    if names.is_empty() {
        return Err(Error::Invalid("the file has no columns".into()));
    }
    let columns = names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let key = format!("c{i}");
            let present: Vec<&Value> = rows.iter().filter_map(|r| r.get(&key)).collect();
            let hint = if present.is_empty() {
                TypeHint::Unknown
            } else if present.iter().all(|v| v.is_number()) {
                TypeHint::Number
            } else if present.iter().all(|v| v.is_boolean()) {
                TypeHint::Boolean
            } else if present
                .iter()
                .all(|v| v.as_str().is_some_and(looks_like_iso_date))
            {
                TypeHint::Date
            } else {
                TypeHint::Text
            };
            column(i, name, hint)
        })
        .collect();
    Ok(ParsedTable {
        columns,
        rows,
        rows_truncated,
        cells_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(p: &ParsedTable) -> Vec<(&str, &str, &str)> {
        p.columns
            .iter()
            .map(|c| (c.col_key.as_str(), c.name.as_str(), c.type_hint.as_str()))
            .collect()
    }

    #[test]
    fn csv_headers_types_and_blanks() {
        let p = parse_csv(b"\xEF\xBB\xBFname,age,ok,when,\nAlice,30,TRUE,2026-01-05,\nBob,2.5,false,2026-01-06T09:00,\n,,,,\n").unwrap();
        assert_eq!(
            keys(&p),
            [
                ("c0", "name", "text"),
                ("c1", "age", "number"),
                ("c2", "ok", "boolean"),
                ("c3", "when", "date"),
                ("c4", "Column 5", "unknown")
            ]
        );
        assert_eq!(
            Value::Object(p.rows[0].clone()),
            json!({"c0":"Alice","c1":30,"c2":true,"c3":"2026-01-05"})
        );
        assert_eq!(Value::Object(p.rows[1].clone())["c1"], json!(2.5));
        assert_eq!(p.rows[2], Map::new());
    }

    #[test]
    fn csv_picks_the_commonest_delimiter_and_ragged_rows() {
        let p = parse_csv(b"a;b\n1;2;3\n").unwrap();
        assert_eq!(p.columns.len(), 3);
        assert_eq!(p.columns[2].name, "Column 3");
        let tabs = parse_csv(b"a\tb\n x \t y \n").unwrap();
        assert_eq!(
            Value::Object(tabs.rows[0].clone()),
            json!({"c0":"x","c1":"y"})
        );
    }

    #[test]
    fn json_shapes() {
        let p = parse_json(br#"[{"a":1,"b":null},{"b":"x","c":{"d":[1]}}]"#).unwrap();
        assert_eq!(
            keys(&p),
            [
                ("c0", "a", "number"),
                ("c1", "b", "text"),
                ("c2", "c", "text")
            ]
        );
        assert_eq!(
            Value::Object(p.rows[1].clone()),
            json!({"c1":"x","c2":"{\"d\":[1]}"})
        );
        let lines = parse_json(b"{\"a\":true}\n\n{\"a\":false}\n").unwrap();
        assert_eq!(keys(&lines), [("c0", "a", "boolean")]);
        let obj = parse_json(br#"{"k":"2026-02-03"}"#).unwrap();
        assert_eq!(keys(&obj), [("c0", "Key", "text"), ("c1", "Value", "date")]);
        let scalars = parse_json(b"[1,2]").unwrap();
        assert_eq!(keys(&scalars), [("c0", "value", "number")]);
    }

    #[test]
    fn typed_cells_follow_the_editor() {
        assert_eq!(cell_from_text(" 42 "), json!(42));
        assert_eq!(cell_from_text("-0"), json!(0));
        assert_eq!(cell_from_text("7.50"), json!(7.5));
        assert_eq!(cell_from_text("   "), json!(""));
        assert_eq!(cell_from_text(" 1e3"), json!(" 1e3"));
        assert_eq!(cell_from_text("1."), json!("1."));
        assert_eq!(cell_from_text("+1"), json!("+1"));
    }

    #[test]
    fn refusals_and_caps() {
        assert!(parse_csv(b"  \n").is_err());
        assert!(parse_json(b"{oops").is_err());
        let wide = (0..=MAX_COLUMNS)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_csv(wide.as_bytes()).is_err());
        let long = format!("h\n{}é\n", "a".repeat(MAX_CELL_BYTES - 1));
        let p = parse_csv(long.as_bytes()).unwrap();
        assert_eq!(p.cells_truncated, 1);
        assert_eq!(p.rows[0]["c0"].as_str().unwrap().len(), MAX_CELL_BYTES - 1);
    }
}
