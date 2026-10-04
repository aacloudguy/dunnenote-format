//! Archiving and retrieving pages, sections and canvases.
//!
//! Archiving marks one row; nothing underneath it changes. Archiving a page or section also
//! writes a snapshot of its subtree to `.archive/<id>.tar.gz` first, which retrieving deletes
//! (`SPEC.md` section 10).

use std::io::{Read, Write as _};
use std::path::Path;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{sync_dir, Writer};
use crate::error::{Error, Result};

/// The folder of archive snapshots inside a notebook.
pub const ARCHIVE_DIR: &str = ".archive";

/// Most bytes a snapshot's members may hold together once unpacked; a larger one is refused
/// unread, so a hostile `.archive/` file cannot exhaust memory (`SPEC.md` §15).
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
/// The snapshot format this library writes and reads.
pub const ARCHIVE_FORMAT_VERSION: u32 = 1;
/// Longest archive note, in bytes.
pub const MAX_ARCHIVE_NOTE_BYTES: usize = 1024;
/// Deepest subtree that can be archived.
const MAX_HIERARCHY_DEPTH: u32 = 64;

/// Why something was archived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveReason {
    Superseded,
    Wrong,
    Irrelevant,
    /// Needs a note saying why.
    Other,
}

impl ArchiveReason {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "superseded" => Some(Self::Superseded),
            "wrong" => Some(Self::Wrong),
            "irrelevant" => Some(Self::Irrelevant),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Superseded => "superseded",
            Self::Wrong => "wrong",
            Self::Irrelevant => "irrelevant",
            Self::Other => "other",
        }
    }
}

/// The note stored with an archive reason: trimmed, required for `other` and refused otherwise.
fn check_note(reason: ArchiveReason, note: Option<&str>) -> Result<Option<String>> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    match (reason, note) {
        (ArchiveReason::Other, None) => {
            return Err(Error::Invalid(
                "archiving for reason \"other\" needs a note".into(),
            ))
        }
        (r, Some(_)) if r != ArchiveReason::Other => {
            return Err(Error::Invalid(format!(
                "a note is only kept with reason \"other\", not \"{}\"",
                r.as_str()
            )))
        }
        _ => {}
    }
    if let Some(n) = note {
        if n.len() > MAX_ARCHIVE_NOTE_BYTES {
            return Err(Error::Invalid(format!(
                "an archive note is at most {MAX_ARCHIVE_NOTE_BYTES} bytes"
            )));
        }
        if n.contains(['\0', '\r', '\n']) {
            return Err(Error::Invalid(
                "an archive note cannot contain line breaks or NUL".into(),
            ));
        }
    }
    Ok(note.map(str::to_string))
}

/// One node in a snapshot's `nodes.jsonl`, fields in this order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivedNode {
    pub id: String,
    pub kind: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub position: String,
    pub is_archived: bool,
    pub is_template: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize)]
struct SnapshotManifest {
    format_version: u32,
    app: String,
    checksum_algorithm: String,
    root_node_id: String,
    entries: Vec<SnapshotEntry>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotEntry {
    path: String,
    size: u64,
    sha256: String,
}

fn tar_member<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    path: &str,
    data: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_size(data.len() as u64);
    header.set_username("")?;
    header.set_groupname("")?;
    builder.append_data(&mut header, path, data)?;
    Ok(())
}

/// A subtree snapshot, byte for byte as DunneNote writes it: a gzip (mtime 0, OS byte 255,
/// level 6) tar of `manifest.json` then `nodes.jsonl` (rows sorted by id), with zeroed owners and
/// times.
pub fn snapshot_bytes(nodes: &[ArchivedNode], root_node_id: &str) -> Result<Vec<u8>> {
    let mut sorted: Vec<&ArchivedNode> = nodes.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut jsonl = Vec::new();
    for n in sorted {
        jsonl.extend(serde_json::to_vec(n).map_err(|e| Error::Malformed(e.to_string()))?);
        jsonl.push(b'\n');
    }
    let manifest = SnapshotManifest {
        format_version: ARCHIVE_FORMAT_VERSION,
        app: "dunnenote".into(),
        checksum_algorithm: "sha256".into(),
        root_node_id: root_node_id.into(),
        entries: vec![SnapshotEntry {
            path: "nodes.jsonl".into(),
            size: jsonl.len() as u64,
            sha256: hex::encode(Sha256::digest(&jsonl)),
        }],
    };
    let manifest = serde_json::to_vec(&manifest).map_err(|e| Error::Malformed(e.to_string()))?;
    let gz = flate2::GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), flate2::Compression::new(6));
    let mut builder = tar::Builder::new(gz);
    tar_member(&mut builder, "manifest.json", &manifest)?;
    tar_member(&mut builder, "nodes.jsonl", &jsonl)?;
    Ok(builder.into_inner()?.finish()?)
}

/// Read a snapshot and check it: only the two regular members, a known format version, and
/// `nodes.jsonl` matching its recorded size and SHA-256. Nothing is unpacked to disk.
pub fn read_snapshot(bytes: &[u8]) -> Result<Vec<ArchivedNode>> {
    let bad = |why: &str| Error::Malformed(format!("archive snapshot: {why}"));
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let (mut manifest, mut nodes) = (None, None);
    let mut total: u64 = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            return Err(bad("unexpected member type"));
        }
        let path = entry.path()?.to_string_lossy().into_owned();
        total = total.saturating_add(entry.size());
        if total > MAX_SNAPSHOT_BYTES {
            return Err(bad("larger than a snapshot can be"));
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(MAX_SNAPSHOT_BYTES)
            .read_to_end(&mut data)?;
        match path.as_str() {
            "manifest.json" => manifest = Some(data),
            "nodes.jsonl" => nodes = Some(data),
            _ => return Err(bad("unexpected member")),
        }
    }
    let manifest: SnapshotManifest =
        serde_json::from_slice(&manifest.ok_or_else(|| bad("no manifest.json"))?)
            .map_err(|e| bad(&e.to_string()))?;
    if manifest.format_version > ARCHIVE_FORMAT_VERSION {
        return Err(bad("made by a newer version"));
    }
    let nodes = nodes.ok_or_else(|| bad("no nodes.jsonl"))?;
    let entry = manifest
        .entries
        .iter()
        .find(|e| e.path == "nodes.jsonl")
        .ok_or_else(|| bad("nodes.jsonl is not listed"))?;
    if entry.size != nodes.len() as u64 || entry.sha256 != hex::encode(Sha256::digest(&nodes)) {
        return Err(bad("nodes.jsonl does not match its checksum"));
    }
    nodes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice(l).map_err(|e| bad(&e.to_string())))
        .collect()
}

/// Where a node's snapshot lives. The id comes from the notebook, which may be hostile, and the
/// schema only checks its length, so it must be a UUID before it becomes part of a path.
fn snapshot_path(root: &Path, id: &str) -> Result<std::path::PathBuf> {
    if !crate::notebook::is_uuid(id) {
        return Err(Error::Malformed(format!("{id:?} is not a UUID")));
    }
    Ok(root.join(ARCHIVE_DIR).join(format!("{id}.tar.gz")))
}

fn write_snapshot(root: &Path, id: &str, bytes: &[u8]) -> Result<()> {
    let path = snapshot_path(root, id)?;
    let dir = root.join(ARCHIVE_DIR);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| Error::Io(e.error))?;
    sync_dir(&dir)
}

impl Writer<'_> {
    /// Archive a page, section or the notebook itself, as DunneNote does: refused when it or an
    /// ancestor is already archived, or (except for the notebook) something under it is. Its
    /// subtree snapshot is written to `.archive/<id>.tar.gz` first; then only its own row is
    /// marked. What is under it, its canvases and the search index are unchanged.
    pub fn archive_node(
        &mut self,
        id: &str,
        reason: ArchiveReason,
        note: Option<&str>,
    ) -> Result<()> {
        let note = check_note(reason, note)?;
        let kind = self.node_kind(id)?;
        if self.archived_in_path(id)? {
            return Err(Error::Invalid(format!(
                "{id} or a section above it is already archived"
            )));
        }
        let mut stmt = self.tx.prepare(
            "WITH RECURSIVE subtree(id, kind, parent_id, name, position, is_archived, is_template, \
               created_at, updated_at, depth) AS ( \
                 SELECT id, kind, parent_id, name, position, is_archived, is_template, created_at, updated_at, 0 \
                   FROM nodes WHERE id = ?1 \
                 UNION ALL \
                 SELECT n.id, n.kind, n.parent_id, n.name, n.position, n.is_archived, n.is_template, \
                        n.created_at, n.updated_at, s.depth + 1 \
                   FROM nodes n JOIN subtree s ON n.parent_id = s.id) \
             SELECT id, kind, parent_id, name, position, is_archived, is_template, created_at, updated_at, depth \
               FROM subtree ORDER BY depth ASC, position ASC",
        )?;
        let rows: Vec<(ArchivedNode, u32)> = stmt
            .query_map([id], |r| {
                Ok((
                    ArchivedNode {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        parent_id: r.get(2)?,
                        name: r.get(3)?,
                        position: r.get(4)?,
                        is_archived: r.get::<_, i64>(5)? != 0,
                        is_template: r.get::<_, i64>(6)? != 0,
                        created_at: r.get(7)?,
                        updated_at: r.get(8)?,
                    },
                    r.get(9)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        if rows.iter().any(|(_, depth)| *depth >= MAX_HIERARCHY_DEPTH) {
            return Err(Error::Invalid(format!(
                "{id} is nested too deeply to archive"
            )));
        }
        if kind != "notebook" {
            if let Some((bad, _)) = rows.iter().skip(1).find(|(n, _)| n.is_archived) {
                return Err(Error::Invalid(format!(
                    "{} under it is already archived; retrieve it first",
                    bad.id
                )));
            }
        }
        let nodes: Vec<ArchivedNode> = rows.into_iter().map(|(n, _)| n).collect();
        write_snapshot(self.root, id, &snapshot_bytes(&nodes, id)?)?;
        self.tx.execute(
            "UPDATE nodes SET is_archived = 1, archive_reason = ?2, archive_note = ?3, \
             archived_at = unixepoch(), updated_at = unixepoch() WHERE id = ?1",
            params![id, reason.as_str(), note],
        )?;
        Ok(())
    }

    /// Retrieve an archived page, section or notebook: clear its archive columns, then delete its
    /// snapshot if the snapshot checks out (a damaged one is left in place).
    pub fn retrieve_node(&mut self, id: &str) -> Result<()> {
        self.node_kind(id)?;
        let archived: i64 =
            self.tx
                .query_row("SELECT is_archived FROM nodes WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?;
        if archived == 0 {
            return Err(Error::Invalid(format!("{id} is not archived")));
        }
        self.tx.execute(
            "UPDATE nodes SET is_archived = 0, archive_reason = NULL, archive_note = NULL, \
             archived_at = NULL, updated_at = unixepoch() WHERE id = ?1",
            [id],
        )?;
        let path = snapshot_path(self.root, id)?;
        if let Ok(bytes) = std::fs::read(&path) {
            if read_snapshot(&bytes).is_ok() {
                let _ = std::fs::remove_file(&path);
            }
        }
        Ok(())
    }

    /// Archive a canvas: it stays on its page, marked archived with the reason.
    pub fn archive_canvas(
        &mut self,
        canvas: &str,
        reason: ArchiveReason,
        note: Option<&str>,
    ) -> Result<()> {
        let note = check_note(reason, note)?;
        let state = self.canvas_lifecycle(canvas)?;
        if state == "archived" {
            return Err(Error::Invalid(format!(
                "canvas {canvas} is already archived"
            )));
        }
        self.tx.execute(
            "UPDATE canvas_instances SET lifecycle = 'archived', lifecycle_reason = ?2, \
             lifecycle_note = ?3, lifecycle_at = unixepoch(), updated_at = unixepoch() WHERE id = ?1",
            params![canvas, reason.as_str(), note],
        )?;
        Ok(())
    }

    /// Retrieve an archived canvas; its reason and note are cleared.
    pub fn retrieve_canvas(&mut self, canvas: &str) -> Result<()> {
        if self.canvas_lifecycle(canvas)? != "archived" {
            return Err(Error::Invalid(format!("canvas {canvas} is not archived")));
        }
        self.tx.execute(
            "UPDATE canvas_instances SET lifecycle = 'active', lifecycle_reason = NULL, \
             lifecycle_note = NULL, lifecycle_at = NULL, updated_at = unixepoch() WHERE id = ?1",
            [canvas],
        )?;
        Ok(())
    }

    fn canvas_lifecycle(&self, canvas: &str) -> Result<String> {
        self.tx
            .query_row(
                "SELECT lifecycle FROM canvas_instances WHERE id = ?1",
                [canvas],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "canvas",
                id: canvas.into(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_follow_the_reason() {
        assert!(check_note(ArchiveReason::Other, Some("  ")).is_err());
        assert_eq!(
            check_note(ArchiveReason::Other, Some(" dup "))
                .unwrap()
                .as_deref(),
            Some("dup")
        );
        assert!(check_note(ArchiveReason::Wrong, Some("x")).is_err());
        assert_eq!(check_note(ArchiveReason::Wrong, Some("   ")).unwrap(), None);
        assert!(check_note(ArchiveReason::Other, Some("a\nb")).is_err());
        assert!(check_note(ArchiveReason::Other, Some(&"x".repeat(1025))).is_err());
    }

    #[test]
    fn snapshots_are_deterministic_and_verify() {
        let n = |id: &str| ArchivedNode {
            id: id.into(),
            kind: "page".into(),
            parent_id: Some("p".into()),
            name: "N".into(),
            position: "80".into(),
            is_archived: false,
            is_template: false,
            created_at: 1,
            updated_at: 2,
        };
        let a = snapshot_bytes(&[n("b"), n("a")], "b").unwrap();
        assert_eq!(a, snapshot_bytes(&[n("a"), n("b")], "b").unwrap());
        assert_eq!(&a[..10], &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255]);
        let back = read_snapshot(&a).unwrap();
        assert_eq!(
            back.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        // nodes.jsonl that no longer matches the manifest's checksum.
        let manifest = {
            let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&a[..]));
            let mut e = ar.entries().unwrap().next().unwrap().unwrap();
            let mut m = Vec::new();
            e.read_to_end(&mut m).unwrap();
            m
        };
        let mut builder = tar::Builder::new(Vec::new());
        tar_member(&mut builder, "manifest.json", &manifest).unwrap();
        tar_member(&mut builder, "nodes.jsonl", b"{}\n").unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        gz.write_all(&builder.into_inner().unwrap()).unwrap();
        assert!(read_snapshot(&gz.finish().unwrap()).is_err());
        assert!(read_snapshot(b"not gzip").is_err());
    }

    #[test]
    fn oversized_snapshots_are_refused_unread() {
        // A header that claims more than a snapshot may hold, with no body behind it.
        let mut header = tar::Header::new_gnu();
        header.set_path("nodes.jsonl").unwrap();
        header.set_size(MAX_SNAPSHOT_BYTES + 1);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
        gz.write_all(header.as_bytes()).unwrap();
        let err = read_snapshot(&gz.finish().unwrap()).unwrap_err();
        assert!(err.to_string().contains("larger than"), "{err}");
    }
}
