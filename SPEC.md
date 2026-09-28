# DunneNote Format 0.18 — draft

> **Draft.** This document is being written from DunneNote's implementation. Sections marked
> *(to be written)* are not yet normative. The schema in `schema/v18.sql` already is.

The key words MUST, MUST NOT, SHOULD and MAY are to be interpreted as described in RFC 2119.

1. Scope and conformance — Reader and Writer levels *(to be written)*
2. Bundle layout: `format.json`, `notebook.db`, `blobs/`, `attachments/`, `.settings/`, `.state/`,
   `.archive/`, the lock file, and which parts are durable *(to be written)*
3. SQLite profile: required PRAGMAs, WAL, the version gate *(to be written)*
4. Schema: `schema/v18.sql` is normative; table-by-table semantics *(to be written)*
5. Identifiers, sibling positions, timestamps — [below](#5-identifiers-sibling-positions-timestamps)
6. Blobs: addressing, sentinel blobs, reference counts — [below](#6-blobs)
7. Canvases: common columns, settings merge rule, reserved settings keys *(to be written)*
8. Payloads: rich text and sketch [below](#8-payloads-rich-text-and-sketch); tables, calendar,
   per-kind settings *(to be written)*
9. Roles: captions, forms, templates *(to be written)*
10. Groups, layering, archive *(to be written)*
11. Tags and metadata, fold v1 *(to be written)*
12. The search index as a derived cache *(to be written)*
13. Writer checklist — [below](#13-writer-checklist)
14. Versioning policy and changelog *(to be written)*

## 5. Identifiers, sibling positions, timestamps

- **Ids.** Every row id, and `format.json`'s `notebook_id`, is a lowercase hyphenated UUID
  (36 characters). Writers SHOULD mint UUIDv7. The root node's id is not the `notebook_id`.
- **Sibling positions** (`nodes.position`) are fractional indexes: the bytes of a `ZenoIndex`
  (the `fractional_index` crate, 1.0.1) followed by one sentinel byte `0x80`, written as lowercase
  hex, at most 256 characters. Siblings sort by plain byte order of the string, and
  `(parent_id, position)` is unique. The first child of an empty parent is `80`; appending gives
  `c080`, `c180`, …; inserting before `80` gives `4080`. A writer MUST produce positions with this
  codec and MUST NOT rewrite existing siblings' positions to make room.
- **Timestamps** are Unix seconds (UTC). Writers MUST take them from SQLite's `unixepoch()` in the
  statement that writes the row, not from their own clock. (`format.json`'s `created_at` is the
  one exception: it is written before the database exists.)
- **Names** of notebooks, sections and pages are 1–255 bytes with no NUL, CR or LF, stored
  exactly as given (no trimming or normalisation).

## 6. Blobs

- A blob is stored at `blobs/sha256/<h[0..2]>/<h[2..4]>/<h>.bin`, where `h` is the lowercase
  SHA-256 of its bytes, with one `blobs` row (`hash`, `size_bytes`).
- **Order.** A writer MUST write the file (to a temporary name in the same folder, synced, then
  renamed into place without replacing an existing file, then the folder synced) *before* the
  row or any canvas that refers to it. A file whose row was never committed is harmless.
- **Reference counts** (`blobs.refcount`) equal the number of canvases whose `source_hash` is the
  blob. The schema's triggers maintain them; writers MUST NOT change `refcount` themselves. A new
  row starts at 0. Because deleting a page cascades to its canvases, the connection MUST have
  `recursive_triggers` on, or counts leak.
- **Sentinel blobs.** Canvases whose content lives in the database still point at a source blob,
  a fixed byte string (the `calnote:` prefix is part of the content and is never renamed):

  | Used by | Bytes | SHA-256 |
  | --- | --- | --- |
  | Rich Text | `calnote:rich_text:blank:v1` | `a08c69405cee37e0daa8cbc60d66009bd8d14e964f6870433981de248ddc1995` |
  | Sketch | `calnote:sketch:blank:v1` | `0b7e1105132b4dca5b3b88bdf5a8ba4e61e4c3e7928dbd87138bf2b5595d5883` |
  | Editable table | `calnote:spreadsheet:blank:v1` | `31dab27d06645fa76fda6ff0d5f22a42387c0970bc458275242ee456e8184e24` |
  | Template placeholder picture | `calnote:canvas:placeholder:v1` | `f999f31fe5e877e891b7f85f4d26559818b8f3f01604900ed6073b9e284eaac4` |

  Sentinels are ordinary blobs (file and row) created the first time they are needed.
- Unused blobs (refcount 0) are reclaimed by DunneNote. Other writers MUST NOT delete blobs.

## 8. Payloads: rich text and sketch

- **Rich text** (`rich_text_instances.data`, `schema_version` 1) is a bare ProseMirror document,
  `{"type":"doc","content":[…]}`, in DunneNote's editor schema:
  - blocks: `paragraph` (`align`), `heading` (`level` 1–3, `align`), `bullet_list`,
    `ordered_list` (`order`), `list_item` (content: a paragraph, then blocks);
  - inline: `text` (non-empty), `numFmt` (`raw`, `format`), `notebook_link` (`canvasId`,
    `labelSnapshot`, `notebookId` — empty for this notebook — and `notebookLabelSnapshot`);
  - marks, in this rank order: `strong`, `em`, `underline`, `strike`, `link` (`href`, `title`),
    `font_family` (`key`), `font_size` (`px`), `text_color` (`color`), `highlight` (`color`);
  - `align` is `left`, `center`, `right` or `justify`; colours are lowercase `#rrggbb` /
    `#rrggbbaa`, `rgb(r, g, b)` or `rgba(r, g, b, a)`; links MUST NOT use `javascript:`, `data:`,
    `vbscript:` or `blob:`.

  A writer MUST NOT store a document with unknown nodes, marks or attributes, empty text nodes or
  content that breaks these rules: DunneNote refuses the first and silently alters the rest. An
  empty canvas holds `{"type":"doc","content":[{"type":"paragraph"}]}`. At most 8 MiB.
- **Sketch** (`sketch_instances.data`, `schema_version` 1) is
  `{"v":1,"strokes":[{"id","points":[{"x","y","p"}],"color","width","tool":"pen"}]}`, keys in
  that order. `x` and `y` are fractions of the canvas frame, clamped to 0–1 and rounded to 4
  places; pressure `p` is in (0, 1], rounded to 2 places with a floor of 0.01. Every stroke has a
  non-empty unique `id`, at least two points, a colour (`#rrggbb` recommended) and a width > 0.
  DunneNote opens a sketch containing any stroke it cannot read **read-only**, so writers MUST NOT
  store one. An empty sketch is `{"v":1,"strokes":[]}`. At most 8 MiB.
- **Picture markup** uses the sketch shape, in the `sketch_instances` row whose `instance_id` is
  the picture's canvas id, with coordinates as fractions of the natural image. No row means no
  markup.

## 13. Writer checklist

A conforming **Writer**:

1. Writes only notebooks whose `PRAGMA user_version` is exactly 18 and whose manifest major
   version is 0. It opens a notebook one version newer read-only, refuses anything newer, and
   refuses older notebooks with advice to open them once in DunneNote.
2. Takes an exclusive lock on `.dunnenote.lock` (`flock` / `LockFileEx`, creating the file if
   needed) for as long as it has the notebook open, and refuses if the lock is held. It never
   deletes the lock file.
3. Sets on every connection: `journal_mode=WAL`, `foreign_keys=ON`, `recursive_triggers=ON`,
   `synchronous=NORMAL`, `busy_timeout=5000` — and checks `recursive_triggers` before writing.
4. Runs `PRAGMA quick_check` and `PRAGMA foreign_key_check` before its first write and refuses a
   notebook that fails either.
5. Makes each logical change in one `BEGIN IMMEDIATE` transaction.
6. Writes blob files before the rows that refer to them (§6) and never changes `refcount`.
7. Takes every timestamp from `unixepoch()` (§5) and mints UUIDs for ids (§5).
8. Places new canvases on the page's top layer: the page's highest `z_index` (0 on an empty
   page) and the next free `z_minor` within it.
9. Stores only payloads that satisfy §8, and settings objects of at most 64 KiB; when changing
   settings it merges into the stored object, keeping keys it does not know.
10. Empties `search_index` (`DELETE FROM search_index`) in the same transaction as any change, so
    DunneNote rebuilds the whole index on its next open. It never leaves the index partly filled:
    DunneNote only rebuilds an empty index.
11. Before committing, checks that every blob's `refcount` equals its canvases and that the files
    of blobs it added exist, and rolls back otherwise.
12. Never deletes content it does not understand, never collects unused blobs, never deletes
    `.archive/` snapshots.

`dunnenote-format` implements this checklist; its conformance suite includes notebooks it writes
being opened by DunneNote's own code, health-checked with no findings, and read back field for
field.

## Notes gathered so far (not yet normative)

Observed in notebooks written by DunneNote itself (see `fixtures/`), to be folded into the
sections above:

- **`.archive/`** holds one compressed snapshot per archived page or section
  (`<node id>.tar.gz`, holding `manifest.json` with SHA-256 checksums and `nodes.jsonl`). The
  live rows stay the source of truth (`nodes.is_archived` and its
  reason columns); retrieving deletes the snapshot. Readers MAY ignore the folder; writers MUST
  NOT delete it.
- **Archiving a page does not change its canvases.** Their `lifecycle` stays `active`; a reader
  that hides archived content checks the page (and its ancestors) as well as each canvas.
- **Tag `source_kind`.** Every canvas, rich text included, is tagged as `instance`
  (`canvas_instances.id`). The value `canvas` is accepted by the schema but refers to a table
  that no longer exists; writers SHOULD NOT use it.
- **Layering.** New canvases go into the top `z_index` (0 on a fresh page) and take the next
  `z_minor`, so a page's canvases share one major layer until the user moves one.
- **Forms.** A field is a canvas whose settings carry `formField` (`name`, and `label`,
  `required`, `labelDisplay` only when not the default). The answers table is an Editable table
  whose settings carry `formTarget: {"ownedByForm": true}`. The page's settings name it with
  `formDestination: {"kind": "canvas", "id": …}` and give the field order in `formTabOrder`.
  Each submission is one table row; the reserved `Submitted` column holds an RFC 3339 time with
  offset. A drawn answer is stored as the cell text `sketch:<id>` pointing at a canvas with
  `hidden: true`, which is real content and not an orphan.
- **Templates.** A template page has `is_template = 1` and its name ends in ` (template)`.
  Pictures in it point at the placeholder sentinel blob with settings `{"placeholder": true}`;
  text canvases keep their content only when their settings carry `templateKeepContent: true`.
- **Blank Editable tables** are 3 columns × 3 rows with column keys `c0`, `c1`, `c2` and names
  `Column 1` … `Column 3`, and `datasets.source_kind` = `paste`. A blank form answers table
  starts with no columns and records `source_kind` = `csv`.
- **Cleared text** in a template (and in pages made from it) is the empty document
  `{"type":"doc","content":[{"type":"paragraph"}]}`.
