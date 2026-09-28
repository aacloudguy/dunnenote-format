# dunnenote-format

The open **DunneNote Format**: a specification and an MIT-licensed Rust library and command-line
tool (`dnfmt`) for reading and writing DunneNote notebooks (`.dunnenote`) without DunneNote.

DunneNote is a commercial, offline-first notebook app from DunneCorp. Its notebooks are plain
folders on your disk: a SQLite database, a content-addressed blob store, and a small manifest.
This project exists so that your notes never depend on one application to stay readable or
editable.

> **Status: pre-release.** The format is published as **DunneNote Format 0.18 — draft**. It will
> be frozen as 1.0 when DunneNote reaches 1.0. Until then every change is versioned and listed in
> the changelog, and this library states exactly which versions it supports.

| Piece | Status |
| --- | --- |
| Canonical schema (`schema/v18.sql`), generated from DunneNote | available |
| Library: open, version gate, read every canvas kind, tags, blobs; verify | available (pre-release) |
| Library: export to JSON, Markdown and CSV | available (pre-release) |
| Library: create notebooks; write sections, pages, rich text, sketches, pictures, picture markup, canvas groups | available (pre-release) |
| Library: write tables (import CSV/JSON, blank Editable tables, edit rows and columns) | available (pre-release) |
| Library: write forms, calendars, tags, archive, templates, captions API | planned |
| `dnfmt inspect`, `ls`, `cat`, `verify`, `export` | available (pre-release) |
| `dnfmt new`, `add-section`, `add-page`, `add-canvas rich-text\|sketch\|picture\|table`, `table` | available (pre-release) |
| `dnfmt tag`, `archive`, `form submit`, `add-canvas` for calendars | planned |
| `SPEC.md` — full specification with a writer checklist | in progress |
| Golden test notebooks produced by DunneNote itself ([fixtures](fixtures/README.md)) | available |

## Try it

```sh
cargo build --release
./target/release/dnfmt inspect ~/Notes/Research.dunnenote
./target/release/dnfmt ls ~/Notes/Research.dunnenote
./target/release/dnfmt cat ~/Notes/Research.dunnenote <page-or-canvas-id>
./target/release/dnfmt verify ~/Notes/Research.dunnenote --full

# Readable copies (the output folder must be new or empty; nothing is overwritten)
./target/release/dnfmt export --md   ~/Notes/Research.dunnenote ~/Desktop/Research-md
./target/release/dnfmt export --csv  ~/Notes/Research.dunnenote ~/Desktop/Research-csv
./target/release/dnfmt export --json ~/Notes/Research.dunnenote ~/Desktop/Research.json
```

- **Markdown**: one `.md` file per page, in folders named after your sections, with pictures
  copied out and sketches drawn as SVG. Archived content is left out unless you add
  `--include-archived`.
- **CSV**: one file per table and one per calendar.
- **JSON**: everything in the notebook as one document (pictures and files are referenced by
  their SHA-256, not embedded). The document shape is versioned (`export_version`).

The reading commands open notebooks read-only and never take DunneNote's lock, so they are safe to
run while the notebook is open in DunneNote. They change nothing in the notebook. Like any SQLite reader, it
may leave the database's two standard companion files, `notebook.db-wal` (empty) and
`notebook.db-shm`, which DunneNote itself creates whenever it opens the notebook.

## Write

```sh
dnfmt new ~/Notes/Trip.dunnenote --name="Summer Trip"      # prints the notebook's root id
S=$(dnfmt add-section ~/Notes/Trip.dunnenote root "Plans")
P=$(dnfmt add-page ~/Notes/Trip.dunnenote "$S" "Day 1")
dnfmt add-canvas ~/Notes/Trip.dunnenote "$P" rich-text notes.md
dnfmt add-canvas ~/Notes/Trip.dunnenote "$P" picture map.png --alt="Route map"
dnfmt add-canvas ~/Notes/Trip.dunnenote "$P" sketch strokes.json --size=400,300
dnfmt add-canvas ~/Notes/Trip.dunnenote "$P" table bookings.csv    # a Data Table
T=$(dnfmt add-canvas ~/Notes/Trip.dunnenote "$P" table)             # an empty Editable table
dnfmt table ~/Notes/Trip.dunnenote "$T" add-row "Column 1=Tent" "Column 2=2"
```

Or from Rust:

```rust
use dunnenote_format::{payload, At, Frame, Notebook, Settings};

let mut nb = Notebook::create("Trip.dunnenote", None)?;
let root = nb.notebook_node()?.id;
nb.write(|w| {
    let page = w.add_page(&root, "Day 1", At::End)?;
    let doc = payload::rich_text_from_markdown("# Day 1\n\nTrain at **9:10**.");
    w.add_rich_text(&page, Frame::PAGE_TEXT, Some(&doc), &Settings::new())?;
    Ok(())
})?;
```

Editing follows the format's [writer checklist](SPEC.md#13-writer-checklist):

- The commands refuse a notebook that is open in DunneNote: close it there first. They lock it
  while they work, so DunneNote cannot open it at the same moment.
- Each command is one transaction: it either happens completely or not at all.
- Rich text and sketches are checked against what DunneNote can display without loss.
- The search index is emptied, and DunneNote rebuilds it the next time it opens the notebook.
- Notebooks written this way are part of the conformance suite: DunneNote's own code opens them,
  runs its full health check (no findings) and reads back exactly what was written.

## Supported versions

This library reads and writes notebooks at **schema 18** (`format_version` `0.18.0`). Notebooks
from newer DunneNote releases open read-only (one version ahead) or are refused. Older notebooks
are refused with a message to open them once in DunneNote, which upgrades them; this library never
upgrades a notebook itself.

## Licence

MIT — see [LICENSE](LICENSE). The specification, the schema and the test notebooks are under the
same licence.
