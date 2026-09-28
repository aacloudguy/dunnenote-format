//! Creating notebooks and changing them.
//!
//! Every change follows the format's writer rules (`SPEC.md`, "Writer checklist"):
//!
//! - Only a notebook at exactly [`SCHEMA_VERSION`](crate::SCHEMA_VERSION) with a `0.x` manifest
//!   is written. Newer and older notebooks are refused.
//! - The writer holds `.dunnenote.lock` exclusively for as long as the notebook is open, so it
//!   never writes while DunneNote has the notebook open (and DunneNote cannot open it meanwhile).
//!   The lock file is never deleted.
//! - Every connection sets DunneNote's PRAGMAs; `recursive_triggers` is checked before each write,
//!   because without it deleting a page would not release its blobs.
//! - Opening for writing runs SQLite's `quick_check` and `foreign_key_check` first and refuses a
//!   damaged notebook.
//! - Each [`Notebook::write`] call is one `BEGIN IMMEDIATE` transaction. Blob files are written
//!   (and synced) before the rows that refer to them. Reference counts are left to the schema's
//!   triggers. Timestamps come from SQLite's `unixepoch()`.
//! - The search index is a cache: a write empties it in the same transaction, and DunneNote
//!   rebuilds it the next time it opens the notebook. (A partly filled index would not be rebuilt.)
//! - Before committing, the writer checks every blob's reference count and that the files of the
//!   blobs it added exist; any failure rolls the whole write back.
//! - Nothing unknown is deleted, and unused blobs are never collected.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::Path;

use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::gate::Compat;
use crate::manifest::{Manifest, FORMAT_DISCRIMINATOR};
use crate::model::{CanvasKind, Settings, Stroke};
use crate::notebook::{blob_path_in, Notebook};
use crate::payload;
use crate::position;
use crate::schema::{create_schema, FORMAT_VERSION, SCHEMA_VERSION};
use crate::settings_keys;

mod forms;
mod tables;
pub use forms::{Submission, Submitted};
pub use tables::{DATASET_SCHEMA_VERSION, TABLE_SIZE};

/// The folder extension every notebook has.
pub const NOTEBOOK_EXT: &str = ".dunnenote";

/// Name of the lock file inside a notebook.
pub const LOCK_FILE: &str = ".dunnenote.lock";

/// Largest settings object on a canvas or group, in bytes of JSON.
pub const MAX_SETTINGS_BYTES: usize = 64 * 1024;

/// Largest picture file DunneNote accepts.
pub const MAX_PICTURE_BYTES: u64 = 50 * 1024 * 1024;
/// Largest picture width or height, in pixels.
pub const MAX_PICTURE_SIDE: u32 = 16_384;
/// Largest picture area, in pixels.
pub const MAX_PICTURE_PIXELS: u64 = 100_000_000;

/// Deepest nesting of canvas groups.
pub const MAX_GROUP_DEPTH: usize = 64;

/// Longest node name, in bytes.
pub const MAX_NAME_BYTES: usize = 255;

/// The `schema_version` DunneNote writes on canvases, their content rows and groups.
pub const CANVAS_SCHEMA_VERSION: i64 = 1;

/// Source blobs for canvases whose content lives in the database. The `calnote:` prefix is part
/// of the content (and so of the hash) and is never renamed.
pub const RICH_TEXT_SENTINEL: &[u8] = b"calnote:rich_text:blank:v1";
pub const SKETCH_SENTINEL: &[u8] = b"calnote:sketch:blank:v1";
pub const SPREADSHEET_SENTINEL: &[u8] = b"calnote:spreadsheet:blank:v1";
pub const PLACEHOLDER_SENTINEL: &[u8] = b"calnote:canvas:placeholder:v1";

/// Width a new picture is limited to when no size is given (its aspect ratio is kept).
pub const DEFAULT_PICTURE_MAX_WIDTH: i64 = 720;

/// Where a canvas sits on its page, in page pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Frame {
    pub fn new(x: i64, y: i64, width: i64, height: i64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    /// DunneNote's first text box on a new page.
    pub const PAGE_TEXT: Frame = Frame {
        x: 40,
        y: 40,
        width: 720,
        height: 320,
    };
}

/// Where a new section or page goes among its siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At<'a> {
    /// After the last sibling.
    End,
    /// Before the first sibling.
    Start,
    /// Directly after this sibling.
    After(&'a str),
    /// Directly before this sibling.
    Before(&'a str),
}

/// A picture's pixel size and type, read from its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureInfo {
    pub width: u32,
    pub height: u32,
    pub mime: &'static str,
}

/// Check that `bytes` are a picture DunneNote accepts (PNG, JPEG, GIF, WebP or AVIF, within its
/// size limits) and return its size and type.
pub fn picture_info(bytes: &[u8]) -> Result<PictureInfo> {
    if bytes.is_empty() {
        return Err(Error::Invalid("the picture file is empty".into()));
    }
    if bytes.len() as u64 > MAX_PICTURE_BYTES {
        return Err(Error::Invalid(format!(
            "the picture is {} bytes; DunneNote accepts at most {MAX_PICTURE_BYTES}",
            bytes.len()
        )));
    }
    let unsupported = || Error::Invalid("not a PNG, JPEG, GIF, WebP or AVIF picture".to_string());
    let head = &bytes[..bytes.len().min(64 * 1024)];
    let mime = match imagesize::image_type(head).map_err(|_| unsupported())? {
        imagesize::ImageType::Png => "image/png",
        imagesize::ImageType::Jpeg => "image/jpeg",
        imagesize::ImageType::Gif => "image/gif",
        imagesize::ImageType::Webp => "image/webp",
        imagesize::ImageType::Heif(c) if format!("{c:?}").eq_ignore_ascii_case("av1") => {
            "image/avif"
        }
        _ => return Err(unsupported()),
    };
    let size = imagesize::blob_size(bytes).map_err(|_| unsupported())?;
    let (w, h) = (
        u32::try_from(size.width).unwrap_or(u32::MAX),
        u32::try_from(size.height).unwrap_or(u32::MAX),
    );
    if w == 0
        || h == 0
        || w > MAX_PICTURE_SIDE
        || h > MAX_PICTURE_SIDE
        || u64::from(w) * u64::from(h) > MAX_PICTURE_PIXELS
    {
        return Err(Error::Invalid(format!(
            "the picture is {w}x{h} pixels; DunneNote accepts up to {MAX_PICTURE_SIDE} on a side \
             and {MAX_PICTURE_PIXELS} in all"
        )));
    }
    Ok(PictureInfo {
        width: w,
        height: h,
        mime,
    })
}

/// A node name DunneNote accepts: 1–255 bytes, no NUL or line breaks. Kept exactly as given.
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Invalid("a name cannot be empty".into()));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(Error::Invalid(format!(
            "a name can be at most {MAX_NAME_BYTES} bytes"
        )));
    }
    if name.contains(['\0', '\n', '\r']) {
        return Err(Error::Invalid(
            "a name cannot contain line breaks or NUL".into(),
        ));
    }
    Ok(())
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// DunneNote's connection settings, applied to every connection that writes.
fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "recursive_triggers", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    Ok(())
}

fn take_lock(root: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::Locked),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// Flush a directory entry to disk (Unix; elsewhere a no-op).
fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// The name DunneNote gives a notebook created at `root`: its folder name without the extension.
pub fn default_name(root: &Path) -> String {
    let base = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = if base.to_lowercase().ends_with(NOTEBOOK_EXT) {
        base[..base.len() - NOTEBOOK_EXT.len()].to_string()
    } else {
        base
    };
    if stem.trim().is_empty() {
        "Untitled Notebook".into()
    } else {
        stem
    }
}

impl Notebook {
    /// Create a new, empty notebook at `root` (which must not exist and must end in
    /// `.dunnenote`) and open it for writing.
    ///
    /// The notebook holds its root node, named `name` or, by default, after the folder. Like a
    /// notebook DunneNote creates, it has no sections yet.
    pub fn create(root: impl AsRef<Path>, name: Option<&str>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let base = root
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !base.ends_with(NOTEBOOK_EXT) || base.len() == NOTEBOOK_EXT.len() {
            return Err(Error::Invalid(format!(
                "a notebook folder name must end in {NOTEBOOK_EXT}"
            )));
        }
        if root.exists() {
            return Err(Error::Invalid(format!("{} already exists", root.display())));
        }
        let name = name
            .map(str::to_string)
            .unwrap_or_else(|| default_name(&root));
        check_name(&name)?;
        std::fs::create_dir(&root)?;
        match Self::populate(&root, &name) {
            Ok(nb) => Ok(nb),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&root);
                Err(e)
            }
        }
    }

    fn populate(root: &Path, name: &str) -> Result<Self> {
        let lock = take_lock(root)?;
        let manifest = Manifest {
            format: FORMAT_DISCRIMINATOR.into(),
            format_version: FORMAT_VERSION.into(),
            notebook_id: new_id(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            created_by: format!("dunnenote-format/{}", env!("CARGO_PKG_VERSION")),
        };
        // Same bytes as DunneNote: pretty-printed, no trailing newline, written to a temporary
        // file, synced, then moved into place without replacing anything.
        let json = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| Error::ManifestInvalid(e.to_string()))?;
        let mut tmp = tempfile::NamedTempFile::new_in(root)?;
        tmp.write_all(&json)?;
        tmp.as_file().sync_all()?;
        tmp.persist_noclobber(root.join("format.json"))
            .map_err(|e| Error::Io(e.error))?;
        sync_dir(root)?;

        let mut conn = Connection::open_with_flags(
            root.join("notebook.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        apply_pragmas(&conn)?;
        create_schema(&mut conn)?;
        conn.execute(
            "INSERT INTO nodes(id, kind, parent_id, name, position, child_count, is_template, \
             created_at, updated_at) VALUES (?1, 'notebook', NULL, ?2, ?3, 0, 0, unixepoch(), unixepoch())",
            params![new_id(), name, position::first()],
        )?;
        Ok(Self {
            root: root.to_path_buf(),
            manifest,
            compat: Compat::Exact,
            schema_version: SCHEMA_VERSION,
            conn,
            lock: Some(lock),
        })
    }

    /// Open a notebook for reading and writing.
    ///
    /// Fails with [`Error::Locked`] while DunneNote (or another writer) has it open, with
    /// [`Error::ReadOnly`] when its version is not exactly the one this library writes, and with
    /// [`Error::Refused`] when SQLite's own checks find damage.
    pub fn open_writable(root: impl AsRef<Path>) -> Result<Self> {
        let probe = Notebook::open(&root)?;
        if !probe.compat.writable() {
            return Err(Error::ReadOnly(format!(
                "it is at schema {}, and this library writes only schema {SCHEMA_VERSION}",
                probe.schema_version
            )));
        }
        let (root, manifest) = (probe.root.clone(), probe.manifest.clone());
        drop(probe);
        let lock = take_lock(&root)?;
        let conn = Connection::open_with_flags(
            root.join("notebook.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        apply_pragmas(&conn)?;
        // Re-read the version under the lock: the notebook may have changed in between.
        let schema_version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if !Compat::classify(schema_version)?.writable() {
            return Err(Error::ReadOnly(format!(
                "it is at schema {schema_version}, and this library writes only schema {SCHEMA_VERSION}"
            )));
        }
        let quick: Vec<String> = conn
            .prepare("PRAGMA quick_check")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if quick != ["ok"] {
            return Err(Error::Refused(format!(
                "SQLite reports damage ({}); run `dnfmt verify --full` and open it in DunneNote",
                quick.first().map(String::as_str).unwrap_or("")
            )));
        }
        let fk: i64 = conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })?;
        if fk > 0 {
            return Err(Error::Refused(format!(
                "{fk} rows point at missing rows; run `dnfmt verify`"
            )));
        }
        Ok(Self {
            root,
            manifest,
            compat: Compat::Exact,
            schema_version,
            conn,
            lock: Some(lock),
        })
    }

    /// Make changes in one transaction. If `f` returns an error, or the checks after it fail,
    /// nothing is changed (blob files already written stay, unreferenced, as DunneNote's own do).
    pub fn write<T>(&mut self, f: impl FnOnce(&mut Writer<'_>) -> Result<T>) -> Result<T> {
        if self.lock.is_none() {
            return Err(Error::ReadOnly(
                "it was opened for reading; use Notebook::open_writable".into(),
            ));
        }
        let recursive: i64 = self
            .conn
            .query_row("PRAGMA recursive_triggers", [], |r| r.get(0))?;
        if recursive != 1 {
            return Err(Error::Refused("recursive_triggers is off".into()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut w = Writer {
            tx,
            root: &self.root,
            new_blobs: Vec::new(),
        };
        let out = f(&mut w)?;
        w.finish()?;
        Ok(out)
    }
}

/// An open write transaction. Obtain one with [`Notebook::write`].
pub struct Writer<'a> {
    tx: Transaction<'a>,
    root: &'a Path,
    new_blobs: Vec<String>,
}

impl Writer<'_> {
    /// Direct access to the transaction, for changes this API does not cover. Changes made
    /// through it must follow the writer rules in `SPEC.md`; the checks at commit still run.
    pub fn transaction(&self) -> &Transaction<'_> {
        &self.tx
    }

    fn finish(self) -> Result<()> {
        // The search index is a cache. Emptying it makes DunneNote rebuild all of it on open.
        self.tx.execute("DELETE FROM search_index", [])?;
        let drifted: Option<String> = self
            .tx
            .query_row(
                "SELECT b.hash FROM blobs b WHERE b.refcount <> \
                 (SELECT count(*) FROM canvas_instances ci WHERE ci.source_hash = b.hash) LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(hash) = drifted {
            return Err(Error::Refused(format!(
                "blob {hash}'s reference count does not match its canvases"
            )));
        }
        for hash in &self.new_blobs {
            if !blob_path_in(self.root, hash)?.is_file() {
                return Err(Error::Refused(format!("blob {hash} has no file")));
            }
        }
        let fk: i64 =
            self.tx
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?;
        if fk > 0 {
            return Err(Error::Refused(format!("{fk} rows point at missing rows")));
        }
        self.tx.commit()?;
        Ok(())
    }

    // ---- blobs ----------------------------------------------------------------------------

    /// Store `bytes` in the blob store (if not already there) and return their SHA-256. The file
    /// is written and synced before its row; the new row's reference count is 0 until a canvas
    /// uses it.
    pub fn put_blob(&mut self, bytes: &[u8]) -> Result<String> {
        let hash = sha256_hex(bytes);
        let path = blob_path_in(self.root, &hash)?;
        let shard = path.parent().expect("blob paths have a parent");
        std::fs::create_dir_all(shard)?;
        match std::fs::metadata(&path) {
            // Already stored (DunneNote dedupes the same way); make sure it is really these bytes.
            Ok(meta) if meta.len() == bytes.len() as u64 => {
                let actual = sha256_hex(&std::fs::read(&path)?);
                if actual != hash {
                    return Err(Error::BlobCorrupt { hash, actual });
                }
            }
            Ok(_) => {
                return Err(Error::Refused(format!(
                    "blob file {} exists with the wrong size",
                    path.display()
                )))
            }
            Err(_) => {
                let mut tmp = tempfile::NamedTempFile::new_in(shard)?;
                tmp.write_all(bytes)?;
                tmp.as_file().sync_all()?;
                match tmp.persist_noclobber(&path) {
                    Ok(_) => {}
                    Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(Error::Io(e.error)),
                }
                sync_dir(shard)?;
            }
        }
        let stored: Option<i64> = self
            .tx
            .query_row(
                "SELECT size_bytes FROM blobs WHERE hash = ?1",
                [&hash],
                |r| r.get(0),
            )
            .optional()?;
        match stored {
            None => {
                self.tx.execute(
                    "INSERT INTO blobs(hash, size_bytes, refcount, deleted_at, created_at, updated_at) \
                     VALUES (?1, ?2, 0, NULL, unixepoch(), unixepoch())",
                    params![hash, bytes.len() as i64],
                )?;
            }
            Some(size) if size != bytes.len() as i64 => {
                return Err(Error::Refused(format!(
                    "blob {hash} is recorded as {size} bytes"
                )))
            }
            Some(_) => {}
        }
        self.new_blobs.push(hash.clone());
        Ok(hash)
    }

    // ---- the page tree --------------------------------------------------------------------

    fn node_kind(&self, id: &str) -> Result<String> {
        self.tx
            .query_row("SELECT kind FROM nodes WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "node",
                id: id.into(),
            })
    }

    /// Whether `id` or any of its ancestors is archived.
    fn archived_in_path(&self, id: &str) -> Result<bool> {
        Ok(self.tx.query_row(
            "WITH RECURSIVE up(id, parent_id, is_archived) AS ( \
                 SELECT id, parent_id, is_archived FROM nodes WHERE id = ?1 \
                 UNION ALL SELECT n.id, n.parent_id, n.is_archived FROM nodes n JOIN up ON n.id = up.parent_id) \
             SELECT coalesce(max(is_archived), 0) FROM up",
            [id],
            |r| r.get::<_, i64>(0),
        )? == 1)
    }

    fn sibling_position(&self, parent: &str, at: At<'_>) -> Result<String> {
        let edge = |order: &str| -> Result<Option<String>> {
            Ok(self
                .tx
                .query_row(
                    &format!(
                        "SELECT position FROM nodes WHERE parent_id = ?1 ORDER BY position {order} LIMIT 1"
                    ),
                    [parent],
                    |r| r.get(0),
                )
                .optional()?)
        };
        let sibling = |id: &str| -> Result<String> {
            self.tx
                .query_row(
                    "SELECT position FROM nodes WHERE id = ?1 AND parent_id = ?2",
                    [id, parent],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| Error::Invalid(format!("{id} is not a child of {parent}")))
        };
        let neighbour = |pos: &str, cmp: &str, order: &str| -> Result<Option<String>> {
            Ok(self
                .tx
                .query_row(
                    &format!(
                        "SELECT position FROM nodes WHERE parent_id = ?1 AND position {cmp} ?2 \
                         ORDER BY position {order} LIMIT 1"
                    ),
                    [parent, pos],
                    |r| r.get(0),
                )
                .optional()?)
        };
        match at {
            At::End => edge("DESC")?
                .map(|p| position::after(&p))
                .unwrap_or_else(|| Ok(position::first())),
            At::Start => edge("ASC")?
                .map(|p| position::before(&p))
                .unwrap_or_else(|| Ok(position::first())),
            At::After(id) => {
                let pos = sibling(id)?;
                match neighbour(&pos, ">", "ASC")? {
                    Some(next) => position::between(&pos, &next),
                    None => position::after(&pos),
                }
            }
            At::Before(id) => {
                let pos = sibling(id)?;
                match neighbour(&pos, "<", "DESC")? {
                    Some(prev) => position::between(&prev, &pos),
                    None => position::before(&pos),
                }
            }
        }
    }

    fn insert_node(&mut self, kind: &str, parent: &str, name: &str, at: At<'_>) -> Result<String> {
        check_name(name)?;
        match self.node_kind(parent)?.as_str() {
            "page" => {
                return Err(Error::Invalid(
                    "a page cannot contain sections or pages".into(),
                ))
            }
            _ if self.archived_in_path(parent)? => {
                return Err(Error::Invalid(format!(
                    "{parent} is archived; retrieve it in DunneNote first"
                )))
            }
            _ => {}
        }
        let id = new_id();
        let position = self.sibling_position(parent, at)?;
        self.tx.execute(
            "INSERT INTO nodes(id, kind, parent_id, name, position, child_count, is_template, \
             created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, unixepoch(), unixepoch())",
            params![id, kind, parent, name, position],
        )?;
        Ok(id)
    }

    /// Add a section (a `group` node) under the notebook or another section.
    pub fn add_section(&mut self, parent: &str, name: &str, at: At<'_>) -> Result<String> {
        self.insert_node("group", parent, name, at)
    }

    /// Add an empty page under the notebook or a section. (DunneNote puts a text box on a page
    /// it creates; add one with [`Writer::add_rich_text`] and [`Frame::PAGE_TEXT`].)
    pub fn add_page(&mut self, parent: &str, name: &str, at: At<'_>) -> Result<String> {
        self.insert_node("page", parent, name, at)
    }

    /// Rename a notebook, section or page.
    pub fn rename(&mut self, id: &str, name: &str) -> Result<()> {
        check_name(name)?;
        self.node_kind(id)?;
        self.tx.execute(
            "UPDATE nodes SET name = ?2, updated_at = unixepoch() WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    /// Merge `patch` into a page's settings (`nodes.settings`) as DunneNote does: a `null` value
    /// removes a key, keys this library does not know are kept, DunneNote's own keys are written
    /// only when not their default, and nothing left means the column is NULL. Returns what is
    /// stored. See [`settings_keys::PAGE_KEYS`].
    pub fn merge_page_settings(
        &mut self,
        page: &str,
        patch: &Settings,
    ) -> Result<Option<Settings>> {
        if self.node_kind(page)? != "page" {
            return Err(Error::Invalid(format!(
                "{page} is not a page; only pages have settings"
            )));
        }
        settings_keys::check_page_patch(patch)?;
        let stored: Option<String> =
            self.tx
                .query_row("SELECT settings FROM nodes WHERE id = ?1", [page], |r| {
                    r.get(0)
                })?;
        let stored = match stored.as_deref().map(serde_json::from_str::<Value>) {
            None => None,
            Some(Ok(Value::Object(map))) => Some(map),
            Some(_) => {
                return Err(Error::Refused(format!(
                    "page {page}'s settings are not a JSON object; not overwriting them"
                )))
            }
        };
        let merged = settings_keys::merge_page(stored.as_ref(), patch);
        let json = merged.as_ref().map(Self::settings_json).transpose()?;
        self.tx.execute(
            "UPDATE nodes SET settings = ?2, updated_at = unixepoch() WHERE id = ?1",
            params![page, json],
        )?;
        Ok(merged)
    }

    // ---- canvases -------------------------------------------------------------------------

    fn canvas_kind(&self, id: &str) -> Result<(CanvasKind, String)> {
        let (kind, page): (String, String) = self
            .tx
            .query_row(
                "SELECT kind, page_id FROM canvas_instances WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "canvas",
                id: id.into(),
            })?;
        let kind = CanvasKind::parse(&kind)
            .ok_or_else(|| Error::Malformed(format!("unknown canvas kind {kind:?}")))?;
        Ok((kind, page))
    }

    fn expect_kind(&self, id: &str, allowed: &[CanvasKind]) -> Result<String> {
        let (kind, page) = self.canvas_kind(id)?;
        if !allowed.contains(&kind) {
            return Err(Error::Invalid(format!(
                "canvas {id} is a {}, not a {}",
                kind.display_name(),
                allowed
                    .iter()
                    .map(|k| k.display_name())
                    .collect::<Vec<_>>()
                    .join(" or ")
            )));
        }
        Ok(page)
    }

    fn settings_json(settings: &Settings) -> Result<String> {
        let json = Value::Object(settings.clone()).to_string();
        if json.len() > MAX_SETTINGS_BYTES {
            return Err(Error::Invalid(format!(
                "settings are {} bytes; DunneNote stores at most {MAX_SETTINGS_BYTES}",
                json.len()
            )));
        }
        Ok(json)
    }

    fn check_frame(frame: Frame) -> Result<()> {
        if frame.width <= 0 || frame.height <= 0 {
            return Err(Error::Invalid(
                "a canvas needs a positive width and height".into(),
            ));
        }
        Ok(())
    }

    /// Insert a canvas on the top layer of `page`: the page's highest `z_index` (0 on an empty
    /// page) and the next free `z_minor` in it, as DunneNote does.
    fn insert_canvas(
        &mut self,
        page: &str,
        kind: CanvasKind,
        source_hash: &str,
        frame: Frame,
        settings: &Settings,
    ) -> Result<String> {
        self.insert_canvas_in_layer(page, kind, source_hash, frame, settings, None)
    }

    /// The page's top layer: its highest `z_index`, or 0 on an empty page.
    fn top_layer(&self, page: &str) -> Result<i64> {
        Ok(self.tx.query_row(
            "SELECT coalesce(max(z_index), 0) FROM canvas_instances WHERE page_id = ?1",
            [page],
            |r| r.get(0),
        )?)
    }

    /// Insert a canvas in layer `z_index` (the top layer when `None`), taking the next free
    /// `z_minor` in it.
    fn insert_canvas_in_layer(
        &mut self,
        page: &str,
        kind: CanvasKind,
        source_hash: &str,
        frame: Frame,
        settings: &Settings,
        z_index: Option<i64>,
    ) -> Result<String> {
        Self::check_frame(frame)?;
        let settings = Self::settings_json(settings)?;
        if self.node_kind(page)? != "page" {
            return Err(Error::Invalid(format!("{page} is not a page")));
        }
        let z_index = match z_index {
            Some(z) => z,
            None => self.top_layer(page)?,
        };
        let id = new_id();
        self.tx.execute(
            "INSERT INTO canvas_instances(id, page_id, kind, source_hash, x, y, width, height, \
             z_index, z_minor, settings, schema_version, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
               (SELECT coalesce(max(z_minor) + 1, 0) FROM canvas_instances WHERE page_id = ?2 AND z_index = ?9), \
               ?10, ?11, unixepoch(), unixepoch())",
            params![
                id,
                page,
                kind.as_str(),
                source_hash,
                frame.x,
                frame.y,
                frame.width,
                frame.height,
                z_index,
                settings,
                CANVAS_SCHEMA_VERSION
            ],
        )?;
        Ok(id)
    }

    /// Add a Rich Text canvas. `doc` defaults to one empty paragraph; it must pass
    /// [`payload::check_rich_text`].
    pub fn add_rich_text(
        &mut self,
        page: &str,
        frame: Frame,
        doc: Option<&Value>,
        settings: &Settings,
    ) -> Result<String> {
        let data = match doc {
            Some(d) => {
                payload::check_rich_text(d)?;
                d.to_string()
            }
            None => payload::EMPTY_RICH_TEXT.to_string(),
        };
        let hash = self.put_blob(RICH_TEXT_SENTINEL)?;
        let id = self.insert_canvas(page, CanvasKind::RichText, &hash, frame, settings)?;
        self.tx.execute(
            "INSERT INTO rich_text_instances(instance_id, data, schema_version, created_at, updated_at) \
             VALUES (?1, ?2, ?3, unixepoch(), unixepoch())",
            params![id, data, CANVAS_SCHEMA_VERSION],
        )?;
        Ok(id)
    }

    /// Replace the document of a Rich Text canvas.
    pub fn set_rich_text(&mut self, canvas: &str, doc: &Value) -> Result<()> {
        self.expect_kind(canvas, &[CanvasKind::RichText])?;
        payload::check_rich_text(doc)?;
        let n = self.tx.execute(
            "UPDATE rich_text_instances SET data = ?2, schema_version = ?3, updated_at = unixepoch() \
             WHERE instance_id = ?1",
            params![canvas, doc.to_string(), CANVAS_SCHEMA_VERSION],
        )?;
        if n == 0 {
            return Err(Error::Malformed(format!(
                "rich text canvas {canvas} has no content row"
            )));
        }
        Ok(())
    }

    /// Add a Sketch canvas. Stroke coordinates are fractions (0..1) of the canvas frame.
    pub fn add_sketch(
        &mut self,
        page: &str,
        frame: Frame,
        strokes: &[Stroke],
        settings: &Settings,
    ) -> Result<String> {
        let data = payload::sketch_json(strokes)?;
        let hash = self.put_blob(SKETCH_SENTINEL)?;
        let id = self.insert_canvas(page, CanvasKind::Sketch, &hash, frame, settings)?;
        self.tx.execute(
            "INSERT INTO sketch_instances(instance_id, data, schema_version, created_at, updated_at) \
             VALUES (?1, ?2, ?3, unixepoch(), unixepoch())",
            params![id, data, CANVAS_SCHEMA_VERSION],
        )?;
        Ok(id)
    }

    fn upsert_strokes(&mut self, canvas: &str, strokes: &[Stroke]) -> Result<()> {
        let data = payload::sketch_json(strokes)?;
        self.tx.execute(
            "INSERT INTO sketch_instances(instance_id, data, schema_version, created_at, updated_at) \
             VALUES (?1, ?2, ?3, unixepoch(), unixepoch()) \
             ON CONFLICT(instance_id) DO UPDATE SET data = excluded.data, \
               schema_version = excluded.schema_version, updated_at = unixepoch()",
            params![canvas, data, CANVAS_SCHEMA_VERSION],
        )?;
        Ok(())
    }

    /// Replace the strokes of a Sketch canvas.
    pub fn set_sketch(&mut self, canvas: &str, strokes: &[Stroke]) -> Result<()> {
        self.expect_kind(canvas, &[CanvasKind::Sketch])?;
        self.upsert_strokes(canvas, strokes)
    }

    /// Replace the markup drawn over a Picture canvas. Coordinates are fractions (0..1) of the
    /// picture's natural image, not of its frame.
    pub fn set_markup(&mut self, picture: &str, strokes: &[Stroke]) -> Result<()> {
        self.expect_kind(picture, &[CanvasKind::Picture])?;
        self.upsert_strokes(picture, strokes)
    }

    /// Add a Picture canvas showing `image` (PNG, JPEG, GIF, WebP or AVIF) with its top-left
    /// corner at `(x, y)`. Without `size`, the frame is the image's own size, scaled down to
    /// [`DEFAULT_PICTURE_MAX_WIDTH`] if wider. Put alt text in `settings` as `alt`.
    pub fn add_picture(
        &mut self,
        page: &str,
        (x, y): (i64, i64),
        size: Option<(i64, i64)>,
        image: &[u8],
        settings: &Settings,
    ) -> Result<String> {
        let info = picture_info(image)?;
        let (width, height) = size.unwrap_or_else(|| {
            let (w, h) = (i64::from(info.width), i64::from(info.height));
            if w <= DEFAULT_PICTURE_MAX_WIDTH {
                (w, h)
            } else {
                let scaled = (h as f64 * DEFAULT_PICTURE_MAX_WIDTH as f64 / w as f64).round();
                (DEFAULT_PICTURE_MAX_WIDTH, (scaled as i64).max(1))
            }
        });
        let hash = self.put_blob(image)?;
        self.insert_canvas(
            page,
            CanvasKind::Picture,
            &hash,
            Frame::new(x, y, width, height),
            settings,
        )
    }

    /// Move or resize a canvas. Its layer is unchanged.
    pub fn set_frame(&mut self, canvas: &str, frame: Frame) -> Result<()> {
        Self::check_frame(frame)?;
        self.canvas_kind(canvas)?;
        self.tx.execute(
            "UPDATE canvas_instances SET x = ?2, y = ?3, width = ?4, height = ?5, \
             updated_at = unixepoch() WHERE id = ?1",
            params![canvas, frame.x, frame.y, frame.width, frame.height],
        )?;
        Ok(())
    }

    /// Merge `patch` into a canvas's settings: each key in `patch` is set, a `null` value removes
    /// the key, and every other key (including ones this library does not know) is kept in place.
    pub fn merge_settings(&mut self, canvas: &str, patch: &Settings) -> Result<Settings> {
        self.canvas_kind(canvas)?;
        let stored: String = self.tx.query_row(
            "SELECT settings FROM canvas_instances WHERE id = ?1",
            [canvas],
            |r| r.get(0),
        )?;
        let mut settings = match serde_json::from_str::<Value>(&stored) {
            Ok(Value::Object(map)) => map,
            _ => {
                return Err(Error::Refused(format!(
                    "canvas {canvas}'s settings are not a JSON object; not overwriting them"
                )))
            }
        };
        for (key, value) in patch {
            if value.is_null() {
                settings.shift_remove(key);
            } else {
                settings.insert(key.clone(), value.clone());
            }
        }
        let json = Self::settings_json(&settings)?;
        self.tx.execute(
            "UPDATE canvas_instances SET settings = ?2, updated_at = unixepoch() WHERE id = ?1",
            params![canvas, json],
        )?;
        Ok(settings)
    }

    // ---- canvas groups --------------------------------------------------------------------

    /// Create a group of canvases on `page`, optionally inside `parent` (a group on the same
    /// page), and move `members` into it. Groups nest at most [`MAX_GROUP_DEPTH`] deep.
    pub fn add_group(
        &mut self,
        page: &str,
        parent: Option<&str>,
        members: &[&str],
    ) -> Result<String> {
        if self.node_kind(page)? != "page" {
            return Err(Error::Invalid(format!("{page} is not a page")));
        }
        if let Some(parent) = parent {
            let depth = self.group_depth(parent, page)?;
            if depth + 1 >= MAX_GROUP_DEPTH {
                return Err(Error::Invalid(format!(
                    "groups nest at most {MAX_GROUP_DEPTH} deep"
                )));
            }
        }
        let id = new_id();
        self.tx.execute(
            "INSERT INTO groups(id, page_id, parent_group_id, settings, schema_version, created_at, updated_at) \
             VALUES (?1, ?2, ?3, '{}', ?4, unixepoch(), unixepoch())",
            params![id, page, parent, CANVAS_SCHEMA_VERSION],
        )?;
        self.set_group(members, Some(&id))?;
        Ok(id)
    }

    /// How many groups enclose `group` (0 for a top-level group); checks it is on `page`.
    fn group_depth(&self, group: &str, page: &str) -> Result<usize> {
        let mut depth = 0;
        let mut current = group.to_string();
        loop {
            let (group_page, parent): (String, Option<String>) = self
                .tx
                .query_row(
                    "SELECT page_id, parent_group_id FROM groups WHERE id = ?1",
                    [&current],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| Error::NotFound {
                    kind: "group",
                    id: current.clone(),
                })?;
            if group_page != page {
                return Err(Error::Invalid(format!(
                    "group {current} is on another page"
                )));
            }
            match parent {
                None => return Ok(depth),
                Some(p) if depth < MAX_GROUP_DEPTH => {
                    depth += 1;
                    current = p;
                }
                Some(_) => return Err(Error::Malformed("groups nest in a cycle".into())),
            }
        }
    }

    /// Put canvases into `group` (a canvas already in another group moves), or take them out of
    /// any group with `None`. All canvases must be on the group's page.
    pub fn set_group(&mut self, canvases: &[&str], group: Option<&str>) -> Result<()> {
        let group_page: Option<String> = match group {
            Some(g) => Some(
                self.tx
                    .query_row("SELECT page_id FROM groups WHERE id = ?1", [g], |r| {
                        r.get(0)
                    })
                    .optional()?
                    .ok_or_else(|| Error::NotFound {
                        kind: "group",
                        id: g.into(),
                    })?,
            ),
            None => None,
        };
        for canvas in canvases {
            let (_, page) = self.canvas_kind(canvas)?;
            if group_page.as_ref().is_some_and(|gp| *gp != page) {
                return Err(Error::Invalid(format!(
                    "canvas {canvas} is not on the group's page"
                )));
            }
            self.tx.execute(
                "UPDATE canvas_instances SET group_id = ?2, updated_at = unixepoch() WHERE id = ?1",
                params![canvas, group],
            )?;
        }
        Ok(())
    }
}
