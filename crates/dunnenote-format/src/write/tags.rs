//! Tags, aliases and typed metadata.
//!
//! Tag names and aliases share one namespace, compared by their fold v1 form. Tags are applied
//! to pages, sections and the notebook (`node`), canvases (`instance`), tables (`dataset`) and
//! table rows (`dataset_row`) (`SPEC.md` section 11).

use rusqlite::{params, OptionalExtension};

use super::{new_id, Writer};
use crate::error::{Error, Result};
use crate::fold::fold;

/// Longest tag name, alias or metadata key, in characters.
pub const MAX_TAG_NAME_CHARS: usize = 255;
/// Longest text metadata value, in characters.
pub const MAX_META_TEXT_CHARS: usize = 1000;

/// What a tag or metadata value can be attached to.
pub const TAGGABLE_KINDS: [&str; 4] = ["node", "instance", "dataset", "dataset_row"];

/// A metadata value. Keys other than the reserved ones hold text.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    Text(String),
    /// `capture_time`: Unix seconds (> 0) and the same instant as ISO 8601 text.
    Datetime {
        epoch_secs: f64,
        iso8601: String,
    },
    /// `geo`: latitude and longitude in degrees.
    Geo {
        lat: f64,
        lon: f64,
    },
}

/// The reserved metadata keys and their types; every other key is text.
pub const RESERVED_META_KEYS: [(&str, &str); 4] = [
    ("capture_time", "datetime"),
    ("geo", "geo"),
    ("place", "text"),
    ("camera", "text"),
];

/// Check a tag name or alias and return it trimmed with its fold.
pub fn check_tag_name(raw: &str) -> Result<(String, String)> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(Error::Invalid("a tag name cannot be blank".into()));
    }
    if name.chars().count() > MAX_TAG_NAME_CHARS {
        return Err(Error::Invalid(format!(
            "a tag name is at most {MAX_TAG_NAME_CHARS} characters"
        )));
    }
    if name.contains(['\0', '\n', '\r']) {
        return Err(Error::Invalid(
            "a tag name cannot contain line breaks or NUL".into(),
        ));
    }
    let folded = fold(name);
    if folded.is_empty() {
        return Err(Error::Invalid(format!(
            "tag name {name:?} folds to nothing"
        )));
    }
    if folded.split('/').any(|seg| seg.trim().is_empty()) {
        return Err(Error::Invalid(format!(
            "tag name {name:?} has an empty part between slashes"
        )));
    }
    Ok((name.to_string(), folded))
}

impl Writer<'_> {
    fn tag_by_folded(&self, folded: &str) -> Result<Option<String>> {
        Ok(self
            .tx
            .query_row(
                "SELECT id FROM tags WHERE name_folded = ?1",
                [folded],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// `(alias id, tag id)` of the alias with this fold.
    fn alias_by_folded(&self, folded: &str) -> Result<Option<(String, String)>> {
        Ok(self
            .tx
            .query_row(
                "SELECT id, tag_id FROM tag_aliases WHERE alias_folded = ?1",
                [folded],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    fn require_tag(&self, id: &str) -> Result<(String, String)> {
        self.tx
            .query_row(
                "SELECT name, name_folded FROM tags WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "tag",
                id: id.into(),
            })
    }

    /// Check that `(kind, id)` names something that exists and can carry tags and metadata.
    fn require_taggable(&self, kind: &str, id: &str) -> Result<()> {
        let table = match kind {
            "node" => "nodes",
            "instance" => "canvas_instances",
            "dataset" => "datasets",
            "dataset_row" => "dataset_rows",
            _ => {
                return Err(Error::Invalid(format!(
                    "{kind:?} cannot be tagged; use node, instance, dataset or dataset_row"
                )))
            }
        };
        let found = self
            .tx
            .query_row(
                &format!("SELECT 1 FROM {table} WHERE id = ?1"),
                [id],
                |_| Ok(()),
            )
            .optional()?;
        found.ok_or_else(|| Error::NotFound {
            kind: "taggable item",
            id: id.into(),
        })
    }

    /// The tag with this name (or an alias of it), created if there is none. Returns its id.
    pub fn tag(&mut self, name: &str) -> Result<String> {
        let (name, folded) = check_tag_name(name)?;
        if let Some(id) = self.tag_by_folded(&folded)? {
            return Ok(id);
        }
        if let Some((_, tag)) = self.alias_by_folded(&folded)? {
            return Ok(tag);
        }
        let id = new_id();
        self.tx.execute(
            "INSERT INTO tags(id, name, name_folded, created_at, updated_at) \
             VALUES (?1, ?2, ?3, unixepoch(), unixepoch())",
            params![id, name, folded],
        )?;
        Ok(id)
    }

    /// Attach a tag. Returns false when it was already attached.
    pub fn apply_tag(&mut self, tag: &str, kind: &str, id: &str) -> Result<bool> {
        self.require_tag(tag)?;
        self.require_taggable(kind, id)?;
        Ok(self.tx.execute(
            "INSERT OR IGNORE INTO item_tags(id, tag_id, source_kind, source_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, unixepoch())",
            params![new_id(), tag, kind, id],
        )? > 0)
    }

    /// Detach a tag. Returns false when it was not attached.
    pub fn remove_tag(&mut self, tag: &str, kind: &str, id: &str) -> Result<bool> {
        Ok(self.tx.execute(
            "DELETE FROM item_tags WHERE tag_id = ?1 AND source_kind = ?2 AND source_id = ?3",
            params![tag, kind, id],
        )? > 0)
    }

    /// Rename a tag. The new name may not be another tag's name or alias; if it is one of this
    /// tag's own aliases, that alias goes.
    pub fn rename_tag(&mut self, tag: &str, name: &str) -> Result<()> {
        let (name, folded) = check_tag_name(name)?;
        self.require_tag(tag)?;
        if self
            .tag_by_folded(&folded)?
            .is_some_and(|other| other != tag)
        {
            return Err(Error::Invalid(format!(
                "another tag is already called {name:?}"
            )));
        }
        if let Some((alias, target)) = self.alias_by_folded(&folded)? {
            if target != tag {
                return Err(Error::Invalid(format!("{name:?} is another tag's alias")));
            }
            self.tx
                .execute("DELETE FROM tag_aliases WHERE id = ?1", [&alias])?;
        }
        self.tx.execute(
            "UPDATE tags SET name = ?1, name_folded = ?2, updated_at = unixepoch() WHERE id = ?3",
            params![name, folded, tag],
        )?;
        Ok(())
    }

    /// Give a tag another name that finds it. The alias may not match any tag or alias.
    pub fn add_tag_alias(&mut self, tag: &str, alias: &str) -> Result<()> {
        let (alias, folded) = check_tag_name(alias)?;
        self.require_tag(tag)?;
        if self.tag_by_folded(&folded)?.is_some() || self.alias_by_folded(&folded)?.is_some() {
            return Err(Error::Invalid(format!(
                "{alias:?} is already a tag name or alias"
            )));
        }
        self.tx.execute(
            "INSERT INTO tag_aliases(id, alias_name, alias_folded, tag_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, unixepoch())",
            params![new_id(), alias, folded, tag],
        )?;
        Ok(())
    }

    /// Remove an alias (matched by fold). Returns false when there was none.
    pub fn remove_tag_alias(&mut self, alias: &str) -> Result<bool> {
        let folded = fold(alias.trim());
        if folded.is_empty() {
            return Ok(false);
        }
        Ok(self
            .tx
            .execute("DELETE FROM tag_aliases WHERE alias_folded = ?1", [&folded])?
            > 0)
    }

    /// Merge `loser` into `winner`: its applications and aliases move over, it is deleted, and
    /// its name becomes an alias of the winner.
    pub fn merge_tags(&mut self, loser: &str, winner: &str) -> Result<()> {
        if loser == winner {
            return Err(Error::Invalid("a tag cannot be merged into itself".into()));
        }
        let (loser_name, loser_folded) = self.require_tag(loser)?;
        let (_, winner_folded) = self.require_tag(winner)?;
        self.tx.execute(
            "UPDATE OR IGNORE item_tags SET tag_id = ?1 WHERE tag_id = ?2",
            params![winner, loser],
        )?;
        self.tx
            .execute("DELETE FROM item_tags WHERE tag_id = ?1", [loser])?;
        self.tx.execute(
            "DELETE FROM tag_aliases WHERE tag_id = ?1 AND alias_folded = ?2",
            params![loser, winner_folded],
        )?;
        self.tx.execute(
            "UPDATE tag_aliases SET tag_id = ?1 WHERE tag_id = ?2",
            params![winner, loser],
        )?;
        self.tx.execute("DELETE FROM tags WHERE id = ?1", [loser])?;
        self.tx.execute(
            "INSERT OR IGNORE INTO tag_aliases(id, alias_name, alias_folded, tag_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, unixepoch())",
            params![new_id(), loser_name, loser_folded, winner],
        )?;
        Ok(())
    }

    /// Delete a tag, its aliases and everywhere it is applied.
    pub fn delete_tag(&mut self, tag: &str) -> Result<()> {
        self.require_tag(tag)?;
        self.tx.execute("DELETE FROM tags WHERE id = ?1", [tag])?;
        Ok(())
    }

    /// Set a metadata key on an item, replacing its value. The key is stored folded; `source` is
    /// the provenance (`user` for a person, `exif`, or `enrich:<name>`).
    pub fn set_meta(
        &mut self,
        kind: &str,
        id: &str,
        key: &str,
        value: &MetaValue,
        source: &str,
    ) -> Result<()> {
        self.require_taggable(kind, id)?;
        let key = check_meta_key(key)?;
        let expected = RESERVED_META_KEYS
            .iter()
            .find(|(k, _)| *k == key)
            .map_or("text", |(_, t)| *t);
        let (text, folded, num, num2): (Option<String>, Option<String>, Option<f64>, Option<f64>) =
            match (value, expected) {
                (MetaValue::Text(raw), "text") => {
                    let t = raw.trim();
                    if t.is_empty() {
                        return Err(Error::Invalid("a metadata value cannot be blank".into()));
                    }
                    if t.chars().count() > MAX_META_TEXT_CHARS {
                        return Err(Error::Invalid(format!(
                            "a metadata value is at most {MAX_META_TEXT_CHARS} characters"
                        )));
                    }
                    if t.contains('\0') {
                        return Err(Error::Invalid("a metadata value cannot contain NUL".into()));
                    }
                    (Some(t.to_string()), Some(fold(t)), None, None)
                }
                (
                    MetaValue::Datetime {
                        epoch_secs,
                        iso8601,
                    },
                    "datetime",
                ) => {
                    let iso = iso8601.trim();
                    if !epoch_secs.is_finite() || *epoch_secs <= 0.0 {
                        return Err(Error::Invalid("a date and time must be after 1970".into()));
                    }
                    if iso.is_empty() || iso.chars().any(char::is_control) {
                        return Err(Error::Invalid("the ISO 8601 text is blank".into()));
                    }
                    (Some(iso.to_string()), None, Some(*epoch_secs), None)
                }
                (MetaValue::Geo { lat, lon }, "geo") => {
                    if !lat.is_finite() || !(-90.0..=90.0).contains(lat) {
                        return Err(Error::Invalid("latitude is between -90 and 90".into()));
                    }
                    if !lon.is_finite() || !(-180.0..=180.0).contains(lon) {
                        return Err(Error::Invalid("longitude is between -180 and 180".into()));
                    }
                    (None, None, Some(*lat), Some(*lon))
                }
                (_, expected) => {
                    return Err(Error::Invalid(format!(
                        "metadata {key:?} holds a {expected} value"
                    )))
                }
            };
        let source = source.trim();
        if source.is_empty()
            || source.chars().count() > 64
            || source.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(Error::Invalid(format!("bad metadata source {source:?}")));
        }
        self.tx.execute(
            "DELETE FROM item_meta WHERE source_kind = ?1 AND source_id = ?2 AND key = ?3",
            params![kind, id, key],
        )?;
        self.tx.execute(
            "INSERT INTO item_meta(id, source_kind, source_id, key, value_text, value_folded, \
             value_num, value_num2, source, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, unixepoch(), unixepoch())",
            params![new_id(), kind, id, key, text, folded, num, num2, source],
        )?;
        Ok(())
    }

    /// Remove a metadata key from an item. Returns false when it had none.
    pub fn remove_meta(&mut self, kind: &str, id: &str, key: &str) -> Result<bool> {
        let key = check_meta_key(key)?;
        Ok(self.tx.execute(
            "DELETE FROM item_meta WHERE source_kind = ?1 AND source_id = ?2 AND key = ?3",
            params![kind, id, key],
        )? > 0)
    }
}

fn check_meta_key(raw: &str) -> Result<String> {
    let key = raw.trim();
    if key.is_empty() || key.chars().count() > MAX_TAG_NAME_CHARS {
        return Err(Error::Invalid(format!(
            "a metadata key is 1 to {MAX_TAG_NAME_CHARS} characters"
        )));
    }
    if key.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(Error::Invalid(
            "a metadata key cannot contain spaces or control characters".into(),
        ));
    }
    let folded = fold(key);
    if folded.is_empty() {
        return Err(Error::Invalid(format!(
            "metadata key {key:?} folds to nothing"
        )));
    }
    Ok(folded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked_like_dunnenote() {
        assert_eq!(
            check_tag_name("  Projects/Q4 ").unwrap(),
            ("Projects/Q4".into(), "projects/q4".into())
        );
        for bad in ["", "   ", "a//b", "a/ /b", "/a", "\u{301}", "a\nb"] {
            assert!(check_tag_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_tag_name(&"é".repeat(255)).is_ok());
        assert!(check_tag_name(&"é".repeat(256)).is_err());
        assert_eq!(check_meta_key("Vendor").unwrap(), "vendor");
        assert!(check_meta_key("two words").is_err());
    }
}
