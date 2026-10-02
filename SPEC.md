# DunneNote Format 0.18 — draft

> **Draft.** This is a 0.x draft, written from DunneNote's implementation. It is normative for
> notebooks at schema 18, and it may change in later 0.x versions (§14). It becomes 1.0 when
> DunneNote reaches 1.0.

The key words MUST, MUST NOT, SHOULD and MAY are to be interpreted as described in RFC 2119.

1. [Scope and conformance](#1-scope-and-conformance)
2. [Bundle layout](#2-bundle-layout)
3. [SQLite profile and the version gate](#3-sqlite-profile-and-the-version-gate)
4. [Schema](#4-schema)
5. [Identifiers, sibling positions, timestamps](#5-identifiers-sibling-positions-timestamps)
6. [Blobs](#6-blobs)
7. [Canvas and page settings](#7-canvas-and-page-settings)
8. [Payloads: rich text and sketch](#8-payloads-rich-text-and-sketch), [tables](#tables),
   [calendars](#calendars)
9. [Roles: captions, forms and templates](#9-roles-captions-forms-and-templates)
10. [Groups, layering, archive](#10-groups-layering-archive)
11. [Tags and metadata, fold v1](#11-tags-and-metadata-fold-v1)
12. [The search index](#12-the-search-index)
13. [Writer checklist](#13-writer-checklist)
14. [Versioning and changelog](#14-versioning-and-changelog)

## 1. Scope and conformance

This document describes a DunneNote notebook on disk: a `.dunnenote` folder (§2) holding a SQLite
database (§3, §4), a content-addressed blob store (§6) and a manifest. It is enough to read
every notebook DunneNote writes at schema 18 and to change one so that DunneNote opens it with no
loss. It does not describe DunneNote's user interface, sync, backup or encryption.

There are two levels of conformance:

- **Reader.** A Reader opens the database read-only, does not take the notebook's lock (§2), and
  so MAY run while DunneNote has the notebook open. It MUST apply the version gate (§3). It MUST
  ignore columns, settings keys, files and folders that it does not know, and MUST NOT treat the
  search index (§12) as content. It SHOULD show archived content (§10) as archived, or leave it
  out, rather than as live content, and MUST NOT treat form carriers (§9) as orphans.
- **Writer.** A Writer is a Reader that also changes notebooks or creates them. It MUST follow
  every rule in §2–§12, and all of them are gathered in the [Writer checklist](#13-writer-checklist).
  A new notebook is created with the schema in §4, `PRAGMA user_version = 18`, a single notebook
  root node, and the manifest described in §2.

The notebooks in `fixtures/` were produced by DunneNote itself and are part of this
specification: a Reader MUST read each one as `fixtures/expected.json` describes. A Writer's
output is conforming when DunneNote opens it, finds nothing to repair, and reads back what was
written.

## 2. Bundle layout

A notebook is a folder, conventionally named `<name>.dunnenote`. The folder's name is not the
notebook's name (that is the root node's `name`, §4).

| Path | What it is | Durable |
| --- | --- | --- |
| `format.json` | The manifest (below) | yes |
| `notebook.db` | The SQLite database (§3, §4) | yes |
| `blobs/sha256/…` | Pictures, imported files and sentinels, addressed by hash (§6) | yes |
| `attachments/<ext>/…` | Files written by forms whose destination is a file (§9) | yes |
| `.settings/settings.json` | Per-notebook preferences | yes |
| `.state/` | This computer's view state, such as the last selected page | no |
| `.archive/` | Snapshots taken when archiving (§10) | no; a copy of rows in the database |
| `notebook.db.bak-v<N>` | DunneNote's copy of the database before an upgrade from schema N | no |
| `notebook.db-wal`, `notebook.db-shm` | SQLite's companion files | temporary |
| `.dunnenote.lock` | The lock file (below) | no |

"Durable" parts are the notebook: a copy of a notebook MUST include them. The others may be left
out of a copy. Readers MUST NOT rely on `.state/`, whose contents change between DunneNote
releases, and Writers MUST NOT change it.

- **`format.json`** is a JSON object with exactly these fields:
  - `format`: always `"dunnenote"`;
  - `format_version`: the version the notebook was *created* at, `"0.<schema>.0"`, for example
    `"0.18.0"`;
  - `notebook_id`: the notebook's permanent identity, a UUID (§5), unchanged when the folder is
    moved or renamed;
  - `created_at`: Unix seconds;
  - `created_by`: the software that created it, `<name>/<version>`.

  It is written once, when the notebook is created, and never changed afterwards: the schema a
  notebook is at *now* is `PRAGMA user_version` (§3). A Writer creating a notebook writes it to a
  temporary file in the folder, syncs it, and renames it into place without replacing an existing
  file.
- **`.settings/settings.json`** is `{"schema_version":1,"payload":{…}}`. The payload holds
  per-notebook preferences, such as whether a new page starts with a heading. Readers MAY
  ignore it. Writers MUST NOT change it.
- **`attachments/<ext>/`** holds files named by DunneNote, lowercased, grouped by extension. At
  schema 18 the only such files are form answers, `attachments/<ext>/form-answers-<page id>.<ext>`
  (`ext` is `json`, `md` or `csv`). Writers MUST NOT change or delete them.
- **The lock file.** `.dunnenote.lock` is an empty file. The process that has the notebook open
  for writing holds an exclusive advisory lock on it (`flock` on Unix, `LockFileEx` on Windows).
  The lock, not the file, is what matters: the file stays after the notebook closes and MUST NOT
  be deleted. A Reader MAY probe the lock to tell the user that DunneNote has the notebook open;
  it MUST NOT create the file to do so.

## 3. SQLite profile and the version gate

- **The database** is SQLite 3 in WAL mode (`journal_mode=WAL`). Every ordinary table is
  `STRICT` (§4), and some checks use the JSON functions, so SQLite 3.38 or later is required.
- **Writer connections** MUST set `journal_mode=WAL`, `foreign_keys=ON`,
  `recursive_triggers=ON`, `synchronous=NORMAL` and `busy_timeout=5000` on every connection. They
  MUST confirm `recursive_triggers` is on before each write, because the reference counts in §6
  depend on it.
- **Reader connections** SHOULD open the database read-only (`SQLITE_OPEN_READ_ONLY`, and
  `PRAGMA query_only=ON`) with a busy timeout. Opening a WAL database read-only may still leave
  an empty `notebook.db-wal` and a `notebook.db-shm` behind. DunneNote creates the same files and
  they are harmless.
- **The version gate.** `PRAGMA user_version` is the notebook's schema version. This document is
  schema **18**.

  | `user_version` | Reader | Writer |
  | --- | --- | --- |
  | 18 | reads | writes |
  | 19 | reads, ignoring columns and tables it does not know | MUST NOT write |
  | 20 or more | MUST refuse | MUST refuse |
  | less than 18 | MUST refuse; DunneNote upgrades it when it next opens the notebook | MUST refuse, and MUST NOT upgrade it |

  A Reader that refuses SHOULD say why: newer notebooks need a newer reader, and older ones need
  to be opened once in DunneNote.
- **The manifest gate.** A Reader MUST refuse a notebook whose `format.json` is missing, is not
  valid JSON, or lacks any of its five fields (§2); whose `format` is not `"dunnenote"`; or whose
  `format_version` is not a semantic version (`major.minor.patch`, each a number without leading
  zeros, optionally followed by `-pre-release` and `+build` parts, as semver.org 2.0 defines)
  with a major version of 0. So `"0.18"`, `"0.18.0.1"` and `"0.x.y"` are refused. A Reader MAY
  also refuse a `notebook_id` that is not 36 characters (§5). A notebook whose
  `format_version` is older than `user_version` is normal: it was created at that version and
  upgraded since.
- **Before the first write,** a Writer takes the lock (§2), reads `user_version` again while
  holding it, and runs `PRAGMA quick_check` (which must return `ok`) and
  `PRAGMA foreign_key_check` (which must return nothing). It MUST refuse a notebook that fails
  any of these. Each change is then one `BEGIN IMMEDIATE` transaction (§13).

## 4. Schema

`schema/v18.sql` is normative. It is generated from a notebook freshly created by DunneNote, and
its SHA-256 is `dcead3a7cff689b43691fc41d2ab4611ce3355d7a2b8f37152e8b7b95186a433`. A Writer
creating a notebook MUST apply it unchanged, in one transaction, to an empty database, then set
`PRAGMA user_version = 18`. Every ordinary table is `STRICT` (the two FTS5 virtual tables of §12
cannot be), and there are no views. The column checks,
foreign keys (most with `ON DELETE CASCADE`) and triggers in the file are part of the format; the
sections below give their meaning.

- **The tree** (`nodes`). Exactly one root, `kind = 'notebook'` with no parent, holds the
  notebook's name. A section is `kind = 'group'` and sits under the notebook or another section.
  A page is `kind = 'page'`, sits under either, and has no children. Siblings are ordered by
  `position` (§5). `child_count` is kept by triggers. A page may be a template (`is_template`,
  §9), may be archived (`is_archived` and the `archive_*` columns, §10) and may have page settings
  (`settings`, §7).
- **Canvases** (`canvas_instances`) are the things on a page: `kind` is `rich_text`, `sketch`,
  `picture`, `calendar`, `database` (a Data Table) or `spreadsheet` (an Editable table). Each
  has a frame in page pixels (`x`, `y`, `width`, `height`), a layer (`z_index`, `z_minor`, §10),
  optional membership of a group (`group_id`), settings (§7), a lifecycle (`lifecycle` and the
  `lifecycle_*` columns, §10), and a `source_hash`: the blob it was made from, or a sentinel
  (§6). `source_hash` never changes after the row is inserted, and a trigger enforces this.
  `schema_version` is 1.
- **Canvas content** is in one table per kind:
  - `rich_text_instances` and `sketch_instances`, keyed by canvas id (§8). A picture's markup is
    also a `sketch_instances` row;
  - `datasets`, `dataset_columns` and `dataset_rows` for tables ([Tables](#tables));
  - `calendar_events` and `calendar_event_attendees` for calendars ([Calendars](#calendars));
  - a picture's content is its source blob.
- **Canvas groups** (`groups`), §10.
- **Blobs** (`blobs`): one row per file in `blobs/`, with its `refcount` (§6). `deleted_at` is set
  by trigger when the count reaches 0, and DunneNote reclaims such blobs later.
- **Tags and metadata** (`tags`, `tag_aliases`, `item_tags`, `item_meta`), §11.
- **The search index** (`search_index`, `search_index_fts`, `search_index_trgm`), §12.

Triggers a Writer relies on:
- `canvas_instances_after_insert` and `canvas_instances_after_delete` keep `blobs.refcount`
  (§6);
- `nodes_child_count_*` keep `child_count`;
- `nodes_parent_kind_check_*` refuse a tree that breaks the rules above;
- `search_index_after_*` keep the two FTS5 tables in step with `search_index`;
- the `*_touch_updated_at` triggers set `updated_at` whenever a row changes and the statement
  did not set it.

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

## 7. Canvas and page settings

- **Canvas settings** (`canvas_instances.settings`) are a JSON object of at most 64 KiB. A writer
  changing them MUST merge: set the keys it changes, remove a key only by name, and keep every
  other key — including keys it does not know — in place. It MUST NOT re-serialize a subset.
- **Page settings** (`nodes.settings`, pages only) are NULL until set. DunneNote writes the keys it
  does not know first, in their stored order, then its own keys in this order, each only when it
  is not the default: `hideCanvasFrames` (`true`), `formTabOrder` (a non-empty array of canvas
  ids), `formFillMode` (`true`), `formDestination`, `formLabelDisplay`, `hideSubmittedColumn`
  (`true`), `dateDisplayFormat` (`iso`, `dmy`, `mdy`, `numeric-dmy` or `numeric-mdy`). When nothing
  is left the column is set back to NULL. The legacy `formTargetId` (a canvas id) is read as a
  canvas `formDestination` when that key is absent, and is never written.
- **Reserved canvas keys.** A reader treats a key whose value does not have the shape below as
  absent. Flags count only as the literal `true`.

  | Key | Value | Meaning |
  | --- | --- | --- |
  | `hidden` | `true` | Not drawn on the page (form answer carriers, §9) |
  | `formField` | `{"name", "label"?, "required"?, "labelDisplay"?}` | A form field (§9) |
  | `formTarget` | `{"ownedByForm"?: true}` | A form's answers table (§9) |
  | `caption` | `{"anchor": <picture id>, "placement"}` | A caption (§9) |
  | `backgroundTransparent` | `true` | No background fill |
  | `frameOutlineHidden` | `true` | No frame outline |
  | `placeholder` | `true` | A template's cleared picture or calendar |
  | `templateKeepContent` | `true` | Keeps its content when its page is made a template |
  | `alt` | string | A picture's alternative text |

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

### Tables

A table canvas is either a **Data Table** (`kind = 'database'`) or an **Editable table**
(`kind = 'spreadsheet'`). Both have exactly one `datasets` row (`instance_id` unique,
`shape = 'table'`, `schema_version` 1) with its `dataset_columns` and `dataset_rows`.

- **The kind decides whether it changes.** A Data Table holds what was imported and MUST NOT be
  edited: no column or row is added, changed or removed. Only an Editable table is edited.
- **Source.** A table is created in one of five ways, and `source_hash` and `source_kind` say
  which:
  - **Imported file** (a Data Table): `source_hash` is the CSV or JSON file itself and
    `source_kind` is `csv` or `json`.
  - **Pasted text** (a Data Table): `source_hash` is the pasted text, exactly as pasted, and
    `source_kind` is `paste`. The text is read by the CSV import rules below.
  - **Empty Editable table:** points at the Editable table sentinel (§6) with
    `source_kind = 'paste'`: three columns `c0`–`c2` named `Column 1`–`Column 3`
    (`type_hint = 'unknown'`, positions 0–2) and three rows whose `cells` are `{}`.
  - **Converted to an Editable table** from a Data Table or another Editable table: shares the
    source's `source_hash`, copies its `shape` and `source_kind`, and copies all of its columns
    (keys, names, type hints, positions) and rows. The source is left as it was.
  - **A form's answers table:** either new, on the Editable table sentinel with no columns, no
    rows and `source_kind = 'csv'`; or made from an existing table, sharing its `source_hash`,
    copying its `shape`, `source_kind` and columns, and starting with no rows.

  So `source_kind` alone does not tell a Data Table from an Editable table; the canvas `kind`
  does.
- **Columns.** `col_key` matches `^[A-Za-z0-9_]+$` and is unique in its table; keys are `c0`,
  `c1`, … and a new column takes one more than the highest numeric `c` key, so a deleted key is
  never reused. `name` is what is shown. `type_hint` is `text`, `number`, `date`, `boolean` or
  `unknown`. Columns display in `position` order, then by `col_key`; positions need not be
  distinct or contiguous. Deleting a column MUST also remove its key from every row's `cells`.
- **Rows.** `cells` is a JSON object mapping column keys to scalars (string, number, boolean or
  null) — never an object or array; a missing key is an empty cell. Rows display in `seq` order.
  Imported rows are numbered 0, 1, …; a new row takes one more than the highest `seq`, and
  deleting a row leaves a gap. A string cell is at most 64 KiB. Deleting a row also deletes the
  `item_tags` and `item_meta` rows with `source_kind = 'dataset_row'` and its id.
- **`row_count`** MUST equal the number of the table's rows; a writer recounts it
  (`SELECT count(*)`) in the same transaction as any row change and bumps `datasets.updated_at`
  on every change.
- **Import rules** (how DunneNote turns a file into columns and rows; a writer that imports MUST
  follow them so its tables read the same):
  - A UTF-8 byte-order mark is dropped; empty or whitespace-only input is refused; at most 5 MiB.
  - CSV: the delimiter is whichever of `,`, `;` and tab occurs most in the first 8 KiB (comma on
    a tie). The first record is the header; fields are trimmed; rows may be ragged and the widest
    sets the column count. A blank header is `Column N`.
  - JSON: an array of objects gives a column per key, in order of first appearance; an array of
    arrays gives positional columns `Column 1`, …; an array of scalars gives one column `value`
    (a single top-level scalar gives one such row);
    one object gives `Key` / `Value` rows; if the file is not one JSON document it is read as JSON
    Lines. Nested objects and arrays are stored as compact JSON text; `null` is omitted.
  - A column's `type_hint` is `number`, `boolean` or `date` when every value present is of that
    type (CSV: text that parses as a 64-bit integer or a finite 64-bit float, so `+5`, `1e5`
    and `.5` count as numbers; `true`/`false` in any case; text starting
    `YYYY-MM-DD` followed by nothing, `T` or a space), `unknown` when it has no values, and `text`
    otherwise. CSV numbers and booleans in such columns are stored as JSON numbers and booleans;
    all else is a string. Empty values are omitted from `cells`.
  - At most 256 columns (more is refused) and 50 000 rows (the rest are dropped); longer strings
    are cut to 64 KiB at a character boundary.
- **Typed cells.** Text typed into an Editable table cell is stored as a number when, trimmed, it
  matches `-?[0-9]+(\.[0-9]+)?` (written as JavaScript writes numbers: `7`, not `7.0`), as `""`
  when blank, and otherwise as the text exactly as typed.

### Calendars

A Calendar canvas (`kind = 'calendar'`) points at the imported `.ics` file itself as its source
blob; its events are rows of `calendar_events` (with `calendar_event_attendees`), derived from
that file once, when it is imported, and never re-derived. Deleting the canvas deletes them.

- **Accepting a file.** At most 5 MiB and not empty; its first 4 KiB MUST start (after an
  optional UTF-8 byte-order mark and whitespace) with `BEGIN:VCALENDAR` in any case; and it MUST
  parse as iCalendar. (DunneNote's parser then refuses a file with a byte-order mark or leading
  whitespace, so in practice the file starts with `BEGIN:VCALENDAR`.) The blob is stored only
  after the file parses.
- **Events.** Every `VEVENT` is one row, in file order; other components are ignored.
  `source_ordinal` is the event's position among the file's `VEVENT`s, counting ones that are
  skipped. An event is skipped when it has no readable `DTSTART`, when it has a `DTEND` that is
  not readable, when its start plus its `DURATION` (or plus one day) does not fit a 64-bit
  timestamp, when its end is before its start, or past 10 000 events.
- **Times** are anchored without applying any time zone; `VTIMEZONE` is not read:

  | `DTSTART` | `start_utc` | `all_day` | `tzid` |
  | --- | --- | --- | --- |
  | a date | 00:00 UTC that day | 1 | NULL |
  | a UTC time (`…Z`) | that instant | 0 | `UTC` |
  | a floating time | the wall-clock time read as UTC | 0 | NULL |
  | `TZID=<zone>` | the wall-clock time read as UTC | 0 | the zone, verbatim (cut to 128 characters) |

  `end_utc` comes from `DTEND` (same rules), else `start_utc` plus `DURATION`
  (`[+-]P<n>W` or `[+-]P[<n>D][T[<n>H][<n>M][<n>S]]`), else one day for an all-day event and zero
  for a timed one. `dtstamp_utc` and `last_modified_utc` are read the same way (zoned and floating
  values as UTC).
- **Text.** `summary`, `location` and `description` are `''` when absent (at most 1024, 1024 and
  16 384 characters); `uid` is NULL when absent (512). `url`, `status` and `rrule_text` are
  trimmed and NULL when empty (2048, 64, 1024). Recurrences are **not expanded**: the event is
  stored once with its `RRULE` text. `organizer_value` is the `ORGANIZER` value and
  `organizer_cn` its `CN` parameter (512 each). `sequence_no` is `SEQUENCE` as an integer.
- **`categories`** is a JSON array of strings — every `CATEGORIES` property split on commas,
  trimmed, blanks dropped, at most 64 of at most 256 characters — or NULL when there are none.
- **`attachments`** is a JSON array of `{"filename","fmttype","uri"}` (in that order, `null` when
  absent; `filename` from `FILENAME`, else `X-FILENAME`), at most 64, or NULL when there are none.
  An inline attachment (`VALUE=BINARY` or `ENCODING=BASE64`) keeps no `uri`: its body is dropped.
- **Attendees:** at most 512 per event, in file order (`ordinal` 0, 1, …): `value` trimmed
  (at most 512 characters), `cn`, `role` and `partstat` from those parameters, trimmed and NULL
  when absent or empty (512, 64 and 64), `rsvp` 1 only when `RSVP=TRUE` (any case). Longer
  values are cut to the limit at a character boundary.
- **Settings.** A calendar starts with settings `{}`. DunneNote keeps its view in
  `displayDayEpoch` (the shown day's local midnight, Unix seconds), `scale` (`day`, `week`,
  `month`, `event`), `layoutMode` (`standard`, `layered`), `displayMinuteOfDay` (0–1439) and
  `displayEventId`.

## 9. Roles: captions, forms and templates

### Captions

A caption is a Rich Text canvas whose settings carry
`caption: {"anchor": <picture canvas id>, "placement": …}` and that is in the same canvas group
as that picture. `placement` is `bottom`,
`top`, `corner-tl`, `corner-tr`, `corner-bl`, `corner-br`, `movie` or `user` (placed by hand).
DunneNote adds one with `backgroundTransparent: true`, 48 px high across the bottom of the
picture (inside its frame), on the layer above the picture (`max(top layer, picture z_index + 1)`),
grouping the two (the picture's existing group, or a new one), and writes the `caption` key last.
A picture may have several captions.

### Forms

- **Fields.** A form is a page. Each canvas whose settings carry `formField` is a field:
  `{"name": …}` then, only when set, `"label"` (omitted when equal to the name), `"required": true`
  and `"labelDisplay"` (`off`, `hover`, `above` or `below`; the page default is
  `formLabelDisplay`). Names and labels are trimmed and at most 128 UTF-16 code units. A table
  cannot be a field. Hidden and archived canvases are not part of the form.
- **Order.** Fields are read in `formTabOrder` order (ids no longer on the page are skipped,
  repeats count once), then the remaining canvases in layer order.
- **Destination.** The page's `formDestination` is `{"kind":"canvas","id"}` (an answers table, or
  a Rich Text canvas the answers are appended to), `{"kind":"file","format"}` or
  `{"kind":"external","format","path"}` (`format` is `json`, `markdown` or `csv`). A new form's
  answers table is an empty Editable table with settings `{"formTarget":{"ownedByForm":true}}`.
- **Submitting to a table** appends exactly one row. The steps, in DunneNote's order:
  1. Refuse when the page has no field; when a field is named `Submitted` (trimmed, any case);
     when two fields share a name ignoring case; when a field's content cannot be read; when a
     required field's answer is empty.
  2. Each field's answer is text: a Rich Text field gives the text of its text nodes, with a
     space after every node that has content, whitespace runs collapsed to one space and the
     result trimmed; a Calendar field gives the displayed day (`displayDayEpoch`, a local
     midnight) as `YYYY-MM-DD`; an empty sketch, or a picture that is a template placeholder,
     gives `""`.
  3. Answers go to the column whose name matches the field's name ignoring case (the first such
     column). When the table's `formTarget` has `ownedByForm`, missing columns are added — type
     `date` for a calendar field, otherwise `text` — and, if absent, a `Submitted` column of type
     `text`; DunneNote asks before adding columns to a table that already has rows. Otherwise a
     missing column refuses the submission.
  4. A drawn answer (a sketch with strokes) or a picture answer is copied into a **carrier**: a
     new canvas on the form's page with settings `{"hidden":true}`, the field's frame and layer
     (`z_index`), holding the sketch document byte for byte, or pointing at the same picture
     blob. The cell holds the token `sketch:<carrier id>` or `picture:<carrier id>`. Carriers are
     real content: readers MUST NOT treat them as orphans, and exports that skip hidden canvases
     still resolve tokens through them.
  5. The row's cells are all strings: the text answers in field order, then `Submitted` (the
     submission time as RFC 3339 with the submitter's UTC offset, e.g.
     `2026-01-05T09:30:00+01:00`), then the carrier tokens.

### Templates

- A template is a page with `is_template = 1`, named `<name> (template)`.
- **Making a template** of a page (one transaction): a new page after the source page's last
  sibling, then a copy of the source's canvas groups (new ids, same settings, parents remapped),
  then of each canvas in layer order (`z_index`, `z_minor`, `created_at`, `id`), each keeping its
  frame and `z_index` and taking the next `z_minor` there, then of the page settings. A canvas
  whose settings have `templateKeepContent: true` is copied as it is. Every other canvas is
  **cleared**:
  - a picture or calendar points at the placeholder sentinel blob (§6) and its settings become
    `{"placeholder": true}` followed by only these of its keys, in this order:
    `frameOutlineHidden`, `frameAppearance`, `backgroundTransparent`, `hidden`, `formField`,
    `templateKeepContent`, then for a picture `renderMode`, `reflow`, `sizingMode`, `rotation`,
    `rotationSnapDeg`, `rotationStepDecimals`, or for a calendar `scale`, `layoutMode`. A cleared
    picture has no markup and a cleared calendar no events;
  - rich text becomes the empty document; a sketch has no strokes;
  - a table keeps its columns, shape, `source_kind` and number of rows, with every row `{}`;
  - any other settings stay as they were, less `scrollTopPx` and `scrollLeftPx`.
- **A new page from a template** is added at the end of the chosen parent, named as the
  template without ` (template)`, and copies every group and canvas as it is — rich text,
  sketches and markup verbatim, tables with all their rows (renumbered from 0) — with
  `is_template = 0`. Calendar events are not copied in either direction.
- **References follow the copies:** a copied caption's `anchor` points at the copied picture; in
  the copied page settings, `formTabOrder` keeps only ids that were copied (mapped to the
  copies), and a canvas `formDestination` (or the legacy `formTargetId`) points at the copy.
  Tags and metadata are not copied.

## 10. Groups, layering, archive

- **Groups** (`groups`): a group belongs to one page and may sit inside another group on the
  same page (`parent_group_id`), at most 64 deep and never in a cycle. A canvas is in at most one
  group (`canvas_instances.group_id`). New groups have settings `{}`.
- **Layering.** Canvases draw in order of `z_index`, then `z_minor`. A new canvas goes into the
  page's top `z_index` (0 on an empty page) and takes the next `z_minor` in it, so a page's
  canvases usually share one layer. Writers MUST keep `(page_id, z_index, z_minor)` unique, but
  the schema does not enforce it (a swap passes through a duplicate inside its transaction), so a
  Reader MUST NOT rely on it: ties draw in order of `created_at`, then `id`.
- **Archiving a page, section or notebook** (`nodes.is_archived`, `archive_reason`,
  `archive_note`, `archived_at`) marks that one row; nothing under it, none of its canvases and
  no search row changes. A reader hiding archived content checks each node's ancestors too.
  - It is refused when the node or an ancestor is already archived, or — except for the
    notebook itself — when anything under it is.
  - `archive_reason` is `superseded`, `wrong`, `irrelevant` or `other`. `other` requires a note;
    the others have none. A note is trimmed, 1–1024 bytes, without NUL, CR or LF.
  - Before marking the row, the writer stores a snapshot of the subtree at
    `.archive/<node id>.tar.gz` (folder mode 0700; written to a temporary file, synced, then
    renamed into place). It is a gzip (mtime 0, OS byte 255) tar of two regular members with
    zeroed owners, mode 0644 and mtime 0:
    - `manifest.json`, whose keys in order are `format_version` (1), `app` (`"dunnenote"`),
      `checksum_algorithm` (`"sha256"`), `root_node_id` and `entries`, an array holding one
      `{"path":"nodes.jsonl","size","sha256"}` object;
    - `nodes.jsonl`, one object per node of the subtree (as it was before archiving), sorted by
      id, with the keys `id`, `kind`, `parent_id`, `name`, `position`, `is_archived`,
      `is_template`, `created_at` and `updated_at` in that order.
  - **Retrieving** clears the four columns and then deletes the snapshot if it reads back
    correctly; a damaged snapshot is left in place. Writers MUST NOT otherwise delete snapshots.
- **Archiving a canvas** sets `lifecycle = 'archived'` with `lifecycle_reason`,
  `lifecycle_note` (same rules) and `lifecycle_at`; retrieving sets `lifecycle = 'active'` and
  clears the other three. An archived canvas stays in place and searchable.

## 11. Tags and metadata, fold v1

- **Fold v1** is the comparison form of tag names, aliases, metadata keys and text values:
  Unicode NFC, then full lowercase (not locale-aware), then NFD, then every combining mark
  removed, then NFC. Nothing is trimmed. Examples: `Téxas` → `texas`, `Straße` → `straße`,
  `İstanbul` → `istanbul`, `Ελλάδα` → `ελλαδα`, `東京タワー` unchanged, a string of only combining
  marks → empty. Writers MUST produce the same folds as DunneNote, whose Unicode tables are
  those of Rust 1.95 and `unicode-normalization` 0.1.24.
- **Tags** (`tags`): `name` is trimmed, 1–255 characters, without NUL, CR or LF; its fold MUST
  be non-empty and have no blank `/`-separated segment (a `/` expresses hierarchy). `name_folded`
  is unique, and a name also MUST NOT fold to any alias. Creating a tag whose name folds to an
  existing tag or alias returns that tag. `color` and `description` are not written.
- **Aliases** (`tag_aliases`) follow the same name rules and MUST NOT fold to any tag name or
  other alias. Renaming a tag to one of its own aliases removes that alias. **Merging** a tag into
  another moves its applications (dropping duplicates) and aliases (dropping one equal to the
  winner's name), deletes it, and adds its old name as an alias of the winner.
- **Applications** (`item_tags`): unique per `(tag_id, source_kind, source_id)`. `source_kind`
  is `node` (pages, sections, the notebook), `instance` (a canvas), `dataset` (a table's data) or
  `dataset_row`; the item MUST exist. `canvas` is legacy and MUST NOT be written.
- **Metadata** (`item_meta`): one value per `(source_kind, source_id, key)` — setting replaces.
  `key` is stored folded (1–255 characters, no whitespace or control characters). Reserved keys
  are typed: `capture_time` (`value_num` = Unix seconds > 0, `value_text` = ISO 8601), `geo`
  (`value_num` = latitude in [-90, 90], `value_num2` = longitude in [-180, 180]), `place` and
  `camera` (text). Every other key is text: `value_text` trimmed, 1–1000 characters, no NUL,
  with `value_folded` its fold. `source` is the provenance (`user`, `exif`, `enrich:<name>`; no
  whitespace, at most 64 characters).

## 12. The search index

- **A derived cache.** `search_index` holds one row per piece of searchable text (a node's name,
  a canvas's text, a table cell or column, a calendar event, a caption, a picture's alt text, a
  tag, a metadata value), identified by `(source_kind, source_id, field)`. `search_index_fts` (words, with
  diacritics folded and prefixes of 2 and 3 characters) and `search_index_trgm` (trigrams) are
  FTS5 external-content indexes over it, kept in step by triggers. All three are derived from
  the rest of the database: they are not content. A Reader MUST NOT treat them as content and
  SHOULD NOT rely on their exact rows, which may change between DunneNote releases.
- **Rebuilding.** When DunneNote opens a notebook whose `search_index` is empty and which has
  content, it rebuilds the whole index. It does **not** rebuild an index that has rows, even
  rows that are out of date.
- **Writers** therefore MUST empty the index (`DELETE FROM search_index`, which also empties the
  FTS5 tables through the triggers) in the same transaction as any change, and MUST NOT write
  index rows themselves. Until DunneNote next opens the notebook, search in DunneNote finds
  nothing in it, and it then finds everything.
- An empty index in a notebook with content is normal, not damage.

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
12. Never deletes content it does not understand, never collects unused blobs, and never deletes
    `.archive/` snapshots except the one a successful retrieve has just read back (§10).
13. Changes only Editable tables, never Data Tables; keeps every row's `cells` to scalars under
    existing column keys; and recounts `datasets.row_count` whenever it adds or removes a row
    ([Tables](#tables)).
14. Writes page settings as §7 describes, and gives every form submission its own carrier canvases
    and one appended row (§9).
15. Folds with fold v1 exactly (§11), and writes the `.archive/` snapshot before marking a node
    archived (§10).

`dunnenote-format` implements this checklist; its conformance suite includes notebooks it writes
being opened by DunneNote's own code, health-checked with no findings, and read back field for
field.

## 14. Versioning and changelog

- **Two numbers.** `PRAGMA user_version` is the schema a notebook is at now (§3). The
  specification is named after it: "DunneNote Format 0.18" describes schema 18. `format.json`'s
  `format_version` records the version a notebook was created at and is informational, apart
  from its major version (§3).
- **Draft until 1.0.** While DunneNote is before 1.0, the format is a 0.x draft. A DunneNote
  release may raise the schema version. When it does, a new version of this document is
  published with the new `schema/v<N>.sql` and a changelog entry listing every change a Reader
  or Writer needs to know about. When DunneNote reaches 1.0, the format at that schema becomes
  **DunneNote Format 1.0**, and from then on changes follow semantic versioning: a new major
  version for a change that an older Reader cannot safely ignore.
- **Tolerance.** A Reader for schema N reads schema N+1 (§3). DunneNote itself writes only its
  own schema, and upgrades older notebooks when it opens them. It keeps the copy
  `notebook.db.bak-v<N>` of the database first.
- **Other versioned shapes**, each changed independently of the schema and only by raising
  its number:

  | Shape | Where | Version |
  | --- | --- | --- |
  | Canvas | `canvas_instances.schema_version`, `groups.schema_version` | 1 |
  | Rich text and sketch payloads | `rich_text_instances.schema_version`, `sketch_instances.schema_version`; the sketch's own `"v"` | 1 |
  | Table | `datasets.schema_version` | 1 |
  | Archive snapshot | `manifest.json`'s `format_version` (§10) | 1 |
  | Notebook preferences | `.settings/settings.json`'s `schema_version` (§2) | 1 |

  A Reader that meets a higher number than it knows SHOULD treat that item as unreadable rather
  than guess.

### Changelog

- **0.18 (draft)**: the first published version. It describes schema 18, the schema of
  DunneNote 0.9.
