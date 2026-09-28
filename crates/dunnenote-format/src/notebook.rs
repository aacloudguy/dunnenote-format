//! Opening a notebook and reading what is in it.

use std::fs::File;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row as SqlRow};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::gate::Compat;
use crate::manifest::Manifest;
use crate::model::*;

/// An open notebook (a `.dunnenote` directory).
///
/// [`Notebook::open`] is read-only and does **not** take the notebook's lock, so a notebook that
/// is open in DunneNote can still be read (SQLite's write-ahead log keeps the reader consistent).
/// Use [`Notebook::locked_by_another_process`] to find out whether DunneNote has it open.
/// [`Notebook::open_writable`] and [`Notebook::create`] take the lock (see [`crate::write`]).
pub struct Notebook {
    pub(crate) root: PathBuf,
    pub(crate) manifest: Manifest,
    pub(crate) compat: Compat,
    pub(crate) schema_version: u32,
    pub(crate) conn: Connection,
    /// Held while the notebook is open for writing. Declared after `conn` so the database
    /// closes before the lock is released.
    pub(crate) lock: Option<File>,
}

impl std::fmt::Debug for Notebook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notebook")
            .field("root", &self.root)
            .field("notebook_id", &self.manifest.notebook_id)
            .field("schema_version", &self.schema_version)
            .finish_non_exhaustive()
    }
}

fn settings_of(raw: Option<String>, what: &str) -> Result<Option<Settings>> {
    match raw {
        None => Ok(None),
        Some(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Ok(Some(map)),
            Ok(_) | Err(_) => Err(Error::Malformed(format!(
                "{what} settings are not a JSON object"
            ))),
        },
    }
}

const NODE_COLS: &str = "id, kind, parent_id, name, position, child_count, is_template, \
    is_archived, archive_reason, archive_note, archived_at, settings, created_at, updated_at";

fn node_from(row: &SqlRow) -> rusqlite::Result<(Node, Option<String>)> {
    let kind: String = row.get("kind")?;
    Ok((
        Node {
            id: row.get("id")?,
            kind: NodeKind::parse(&kind).unwrap_or(NodeKind::Page),
            parent_id: row.get("parent_id")?,
            name: row.get("name")?,
            position: row.get("position")?,
            child_count: row.get("child_count")?,
            is_template: row.get::<_, i64>("is_template")? == 1,
            is_archived: row.get::<_, i64>("is_archived")? == 1,
            archive_reason: row.get("archive_reason")?,
            archive_note: row.get("archive_note")?,
            archived_at: row.get("archived_at")?,
            settings: None,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        },
        row.get("settings")?,
    ))
}

const CANVAS_COLS: &str = "id, page_id, kind, source_hash, x, y, width, height, z_index, \
    z_minor, settings, schema_version, group_id, lifecycle, lifecycle_reason, lifecycle_note, \
    lifecycle_at, created_at, updated_at";

const CANVAS_ORDER: &str = "ORDER BY z_index, z_minor, created_at, id";

fn canvas_from(row: &SqlRow) -> rusqlite::Result<(Canvas, String, String)> {
    Ok((
        Canvas {
            id: row.get("id")?,
            page_id: row.get("page_id")?,
            kind: CanvasKind::RichText,
            source_hash: row.get("source_hash")?,
            x: row.get("x")?,
            y: row.get("y")?,
            width: row.get("width")?,
            height: row.get("height")?,
            z_index: row.get("z_index")?,
            z_minor: row.get("z_minor")?,
            settings: Settings::new(),
            schema_version: row.get("schema_version")?,
            group_id: row.get("group_id")?,
            lifecycle: row.get("lifecycle")?,
            lifecycle_reason: row.get("lifecycle_reason")?,
            lifecycle_note: row.get("lifecycle_note")?,
            lifecycle_at: row.get("lifecycle_at")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        },
        row.get("kind")?,
        row.get("settings")?,
    ))
}

impl Notebook {
    /// Open a notebook for reading.
    ///
    /// Fails if `root` is not a notebook, its manifest is invalid, or its schema version is not
    /// one this library can read (see [`crate::gate`]).
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        if !root.is_dir() {
            return Err(Error::NotANotebook(root, "not a directory"));
        }
        if !root.join("format.json").is_file() {
            return Err(Error::NotANotebook(root, "no format.json"));
        }
        let db_path = root.join("notebook.db");
        if !db_path.is_file() {
            return Err(Error::NotANotebook(root, "no notebook.db"));
        }
        let manifest = Manifest::read(&root)?;
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "query_only", "ON")?;
        let schema_version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let compat = Compat::classify(schema_version)?;
        Ok(Self {
            root,
            manifest,
            compat,
            schema_version,
            conn,
            lock: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn compat(&self) -> Compat {
        self.compat
    }
    /// The database's `PRAGMA user_version`.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }
    /// Direct read-only access to the database, for queries this API does not cover.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Whether this handle was opened for writing (and holds the notebook's lock).
    pub fn is_writable(&self) -> bool {
        self.lock.is_some()
    }

    /// Whether another process (normally DunneNote) holds the notebook's lock. Never creates the
    /// lock file. Always `false` for a handle that holds the lock itself.
    pub fn locked_by_another_process(&self) -> Result<bool> {
        if self.lock.is_some() {
            return Ok(false);
        }
        let path = self.root.join(".dunnenote.lock");
        if !path.is_file() {
            return Ok(false);
        }
        // std's file locks are `flock` on Unix and `LockFileEx` on Windows, the same primitives
        // DunneNote uses, so a shared probe fails exactly while DunneNote holds its lock.
        let file = File::open(&path)?;
        match file.try_lock_shared() {
            Ok(()) => {
                file.unlock()?;
                Ok(false)
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(true),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }

    // ---- the page tree ----------------------------------------------------------------------

    fn nodes_where(&self, clause: &str, param: Option<&str>) -> Result<Vec<Node>> {
        let sql = format!("SELECT {NODE_COLS} FROM nodes WHERE {clause}");
        let mut stmt = self.conn.prepare(&sql)?;
        let raw: Vec<(Node, Option<String>)> = match param {
            Some(p) => stmt
                .query_map([p], node_from)?
                .collect::<rusqlite::Result<_>>()?,
            None => stmt
                .query_map([], node_from)?
                .collect::<rusqlite::Result<_>>()?,
        };
        raw.into_iter()
            .map(|(mut node, settings)| {
                node.settings = settings_of(settings, "page")?;
                Ok(node)
            })
            .collect()
    }

    /// The notebook's root node. A valid notebook has exactly one.
    pub fn notebook_node(&self) -> Result<Node> {
        let mut roots = self.nodes_where("parent_id IS NULL ORDER BY position", None)?;
        match roots.len() {
            1 => Ok(roots.remove(0)),
            n => Err(Error::Malformed(format!(
                "expected one notebook root, found {n}"
            ))),
        }
    }

    pub fn node(&self, id: &str) -> Result<Node> {
        self.nodes_where("id = ?1", Some(id))?
            .pop()
            .ok_or_else(|| Error::NotFound {
                kind: "node",
                id: id.into(),
            })
    }

    /// Children of a notebook or section group, in sidebar order.
    pub fn children(&self, parent_id: &str) -> Result<Vec<Node>> {
        self.nodes_where("parent_id = ?1 ORDER BY position ASC", Some(parent_id))
    }

    /// Every node, depth-first in sidebar order, with its depth (the notebook itself is 0).
    pub fn walk(&self) -> Result<Vec<(usize, Node)>> {
        let root = self.notebook_node()?;
        let mut out = Vec::new();
        let mut stack = vec![(0usize, root)];
        while let Some((depth, node)) = stack.pop() {
            let kids = if node.kind == NodeKind::Page {
                Vec::new()
            } else {
                self.children(&node.id)?
            };
            out.push((depth, node));
            for kid in kids.into_iter().rev() {
                stack.push((depth + 1, kid));
            }
        }
        Ok(out)
    }

    /// Every page, in sidebar order.
    pub fn pages(&self) -> Result<Vec<Node>> {
        Ok(self
            .walk()?
            .into_iter()
            .map(|(_, n)| n)
            .filter(|n| n.kind == NodeKind::Page)
            .collect())
    }

    // ---- canvases ---------------------------------------------------------------------------

    fn canvases_where(&self, clause: &str, param: &str) -> Result<Vec<Canvas>> {
        let sql = format!("SELECT {CANVAS_COLS} FROM canvas_instances WHERE {clause}");
        let mut stmt = self.conn.prepare(&sql)?;
        let raw: Vec<(Canvas, String, String)> = stmt
            .query_map([param], canvas_from)?
            .collect::<rusqlite::Result<_>>()?;
        raw.into_iter()
            .map(|(mut c, kind, settings)| {
                c.kind = CanvasKind::parse(&kind)
                    .ok_or_else(|| Error::Malformed(format!("unknown canvas kind {kind:?}")))?;
                c.settings = settings_of(Some(settings), "canvas")?.unwrap_or_default();
                Ok(c)
            })
            .collect()
    }

    /// Every canvas on a page, bottom layer first. Includes archived and hidden canvases.
    pub fn canvases(&self, page_id: &str) -> Result<Vec<Canvas>> {
        self.canvases_where(&format!("page_id = ?1 {CANVAS_ORDER}"), page_id)
    }

    pub fn canvas(&self, id: &str) -> Result<Canvas> {
        self.canvases_where("id = ?1", id)?
            .pop()
            .ok_or_else(|| Error::NotFound {
                kind: "canvas",
                id: id.into(),
            })
    }

    /// The document of a Rich Text canvas.
    pub fn rich_text(&self, canvas_id: &str) -> Result<Option<RichText>> {
        self.conn
            .query_row(
                "SELECT data, schema_version, updated_at FROM rich_text_instances WHERE instance_id = ?1",
                [canvas_id],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .map(|(data, schema_version, updated_at)| {
                let doc: Value = serde_json::from_str(&data)
                    .map_err(|e| Error::Malformed(format!("rich text is not JSON: {e}")))?;
                Ok(RichText {
                    doc,
                    schema_version,
                    updated_at,
                })
            })
            .transpose()
    }

    /// Strokes of a Sketch canvas, or the markup layer of a Picture canvas.
    pub fn sketch(&self, canvas_id: &str) -> Result<Option<Sketch>> {
        self.conn
            .query_row(
                "SELECT data, schema_version, updated_at FROM sketch_instances WHERE instance_id = ?1",
                [canvas_id],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .map(|(data, schema_version, updated_at)| {
                let data: Value = serde_json::from_str(&data)
                    .map_err(|e| Error::Malformed(format!("sketch is not JSON: {e}")))?;
                Ok(Sketch {
                    data,
                    schema_version,
                    updated_at,
                })
            })
            .transpose()
    }

    /// The table behind a Data Table or Editable table canvas.
    pub fn dataset(&self, canvas_id: &str) -> Result<Option<Dataset>> {
        let head = self
            .conn
            .query_row(
                "SELECT id, shape, source_kind, row_count, schema_version FROM datasets WHERE instance_id = ?1",
                [canvas_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, shape, source_kind, row_count, schema_version)) = head else {
            return Ok(None);
        };
        let columns = self
            .conn
            .prepare(
                "SELECT id, col_key, name, type_hint, position FROM dataset_columns \
                 WHERE dataset_id = ?1 ORDER BY position, col_key",
            )?
            .query_map([&id], |r| {
                Ok(Column {
                    id: r.get(0)?,
                    col_key: r.get(1)?,
                    name: r.get(2)?,
                    type_hint: r.get(3)?,
                    position: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let raw_rows = self
            .conn
            .prepare(
                "SELECT id, seq, cells FROM dataset_rows WHERE dataset_id = ?1 ORDER BY seq, id",
            )?
            .query_map([&id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let rows = raw_rows
            .into_iter()
            .map(
                |(id, seq, cells)| match serde_json::from_str::<Value>(&cells) {
                    Ok(Value::Object(cells)) => Ok(Row { id, seq, cells }),
                    _ => Err(Error::Malformed(
                        "table row cells are not a JSON object".into(),
                    )),
                },
            )
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Dataset {
            id,
            instance_id: canvas_id.into(),
            shape,
            source_kind,
            row_count,
            schema_version,
            columns,
            rows,
        }))
    }

    /// Events of a Calendar canvas, in start order.
    pub fn calendar_events(&self, canvas_id: &str) -> Result<Vec<CalendarEvent>> {
        let mut events = self
            .conn
            .prepare(
                "SELECT id, instance_id, uid, summary, location, description, start_utc, end_utc, \
                 all_day, tzid, source_ordinal, url, organizer_value, organizer_cn, status, \
                 categories, rrule_text FROM calendar_events WHERE instance_id = ?1 \
                 ORDER BY start_utc, source_ordinal, id",
            )?
            .query_map([canvas_id], |r| {
                Ok(CalendarEvent {
                    id: r.get(0)?,
                    instance_id: r.get(1)?,
                    uid: r.get(2)?,
                    summary: r.get(3)?,
                    location: r.get(4)?,
                    description: r.get(5)?,
                    start_utc: r.get(6)?,
                    end_utc: r.get(7)?,
                    all_day: r.get::<_, i64>(8)? == 1,
                    tzid: r.get(9)?,
                    source_ordinal: r.get(10)?,
                    url: r.get(11)?,
                    organizer_value: r.get(12)?,
                    organizer_cn: r.get(13)?,
                    status: r.get(14)?,
                    categories: r.get(15)?,
                    rrule_text: r.get(16)?,
                    attendees: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT ordinal, value, cn, role, partstat, rsvp FROM calendar_event_attendees \
             WHERE event_id = ?1 ORDER BY ordinal",
        )?;
        for event in &mut events {
            event.attendees = stmt
                .query_map([&event.id], |r| {
                    Ok(Attendee {
                        ordinal: r.get(0)?,
                        value: r.get(1)?,
                        cn: r.get(2)?,
                        role: r.get(3)?,
                        partstat: r.get(4)?,
                        rsvp: r.get::<_, i64>(5)? == 1,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(events)
    }

    /// Canvas groups on a page.
    pub fn groups(&self, page_id: &str) -> Result<Vec<Group>> {
        let raw = self
            .conn
            .prepare(
                "SELECT id, page_id, parent_group_id, settings, schema_version FROM groups \
                 WHERE page_id = ?1 ORDER BY created_at, id",
            )?
            .query_map([page_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raw.into_iter()
            .map(|(id, page_id, parent_group_id, settings, schema_version)| {
                Ok(Group {
                    id,
                    page_id,
                    parent_group_id,
                    settings: settings_of(Some(settings), "group")?.unwrap_or_default(),
                    schema_version,
                })
            })
            .collect()
    }

    // ---- tags and metadata ------------------------------------------------------------------

    fn tags_query(&self, sql: &str, params: &[&str]) -> Result<Vec<Tag>> {
        let mut tags = self
            .conn
            .prepare(sql)?
            .query_map(rusqlite::params_from_iter(params), |r| {
                Ok(Tag {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    name_folded: r.get(2)?,
                    color: r.get(3)?,
                    description: r.get(4)?,
                    aliases: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT alias_name FROM tag_aliases WHERE tag_id = ?1 ORDER BY alias_folded",
        )?;
        for tag in &mut tags {
            tag.aliases = stmt
                .query_map([&tag.id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(tags)
    }

    /// Every tag in the notebook, with its aliases.
    pub fn tags(&self) -> Result<Vec<Tag>> {
        self.tags_query(
            "SELECT id, name, name_folded, color, description FROM tags ORDER BY name_folded",
            &[],
        )
    }

    /// Tags on one item. `source_kind` is `node`, `canvas`, `instance`, `dataset` or `dataset_row`.
    pub fn tags_of(&self, source_kind: &str, source_id: &str) -> Result<Vec<Tag>> {
        self.tags_query(
            "SELECT t.id, t.name, t.name_folded, t.color, t.description FROM item_tags it \
             JOIN tags t ON t.id = it.tag_id \
             WHERE it.source_kind = ?1 AND it.source_id = ?2 ORDER BY t.name_folded",
            &[source_kind, source_id],
        )
    }

    /// Typed metadata on one item.
    pub fn meta_of(&self, source_kind: &str, source_id: &str) -> Result<Vec<Meta>> {
        Ok(self
            .conn
            .prepare(
                "SELECT key, value_text, value_num, value_num2, source FROM item_meta \
                 WHERE source_kind = ?1 AND source_id = ?2 ORDER BY key, id",
            )?
            .query_map([source_kind, source_id], |r| {
                Ok(Meta {
                    key: r.get(0)?,
                    value_text: r.get(1)?,
                    value_num: r.get(2)?,
                    value_num2: r.get(3)?,
                    source: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- blobs ------------------------------------------------------------------------------

    pub fn blob_info(&self, hash: &str) -> Result<BlobInfo> {
        self.conn
            .query_row(
                "SELECT hash, size_bytes, refcount FROM blobs WHERE hash = ?1",
                [hash],
                |r| {
                    Ok(BlobInfo {
                        hash: r.get(0)?,
                        size_bytes: r.get(1)?,
                        refcount: r.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "blob",
                id: hash.into(),
            })
    }

    /// Where a blob's bytes live: `blobs/sha256/<aa>/<bb>/<hash>.bin`.
    pub fn blob_path(&self, hash: &str) -> Result<PathBuf> {
        blob_path_in(&self.root, hash)
    }

    /// A blob's bytes, checked against its hash.
    pub fn read_blob(&self, hash: &str) -> Result<Vec<u8>> {
        let bytes = std::fs::read(self.blob_path(hash)?)?;
        let actual = hex::encode(Sha256::digest(&bytes));
        if actual != hash {
            return Err(Error::BlobCorrupt {
                hash: hash.into(),
                actual,
            });
        }
        Ok(bytes)
    }

    // ---- summary ----------------------------------------------------------------------------

    pub fn counts(&self) -> Result<Counts> {
        let one = |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, [], |r| r.get(0))?) };
        let by_kind = self
            .conn
            .prepare("SELECT kind, count(*) FROM canvas_instances GROUP BY kind ORDER BY kind")?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Counts {
            sections: one("SELECT count(*) FROM nodes WHERE kind = 'group'")?,
            pages: one("SELECT count(*) FROM nodes WHERE kind = 'page'")?,
            canvases: one("SELECT count(*) FROM canvas_instances")?,
            canvases_by_kind: by_kind,
            archived_pages: one(
                "SELECT count(*) FROM nodes WHERE kind = 'page' AND is_archived = 1",
            )?,
            archived_canvases: one(
                "SELECT count(*) FROM canvas_instances WHERE lifecycle = 'archived'",
            )?,
            templates: one("SELECT count(*) FROM nodes WHERE is_template = 1")?,
            tags: one("SELECT count(*) FROM tags")?,
            blobs: one("SELECT count(*) FROM blobs")?,
            blob_bytes: one("SELECT coalesce(sum(size_bytes), 0) FROM blobs")?,
        })
    }
}

/// Where a blob's bytes live under a notebook root.
pub(crate) fn blob_path_in(root: &Path, hash: &str) -> Result<PathBuf> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Malformed(format!(
            "{hash:?} is not a lowercase SHA-256 hex digest"
        )));
    }
    Ok(root
        .join("blobs")
        .join("sha256")
        .join(&hash[0..2])
        .join(&hash[2..4])
        .join(format!("{hash}.bin")))
}
