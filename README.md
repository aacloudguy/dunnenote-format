# dunnenote-format

The open **DunneNote Format**: a specification and an MIT-licensed Rust library and command-line
tool (`dnfmt`) for reading and writing DunneNote notebooks (`.dunnenote`) without DunneNote.

DunneNote is a commercial, offline-first notebook app from Dunne, Corp. (DunneCorp). Its notebooks are plain
folders on your disk: a SQLite database, a content-addressed blob store, and a small manifest.
This project exists so that your notes never depend on one application to stay readable or
editable.

> **Status: 0.1.0, the first release.** The format is published as **DunneNote Format 0.18 —
> draft**. It will be frozen as 1.0 when DunneNote reaches 1.0. Until then every format change is
> versioned and listed in [SPEC.md §14](SPEC.md#14-versioning-and-changelog), every library
> change in [CHANGELOG.md](CHANGELOG.md), and this library states exactly which versions it
> supports.

| Piece | Status |
| --- | --- |
| Canonical schema (`schema/v18.sql`), generated from DunneNote | available |
| Library: open, version gate, read every canvas kind, tags, blobs; verify | available |
| Library: export to JSON, Markdown and CSV | available |
| Library: create notebooks; write sections, pages, rich text, sketches, pictures, picture markup, canvas groups | available |
| Library: write tables (import CSV/JSON, blank Editable tables, edit rows and columns) | available |
| Library: forms (fields, new form, submit to an answers table with carriers), captions, page settings | available |
| Library: templates (make, new from), archive and retrieve (with `.archive/` snapshots), tags, aliases, metadata | available |
| Library: calendars (`.ics` import with events and attendees, as DunneNote reads them) | available |
| `dnfmt inspect`, `ls`, `cat`, `verify`, `export` | available |
| `dnfmt new`, `add-section`, `add-page`, `add-canvas rich-text\|sketch\|picture\|table\|calendar`, `table`, `form`, `caption`, `template`, `archive`, `retrieve`, `tag`, `meta` | available |
| `SPEC.md` — the full specification, with a writer checklist | available (0.18 draft) |
| Golden test notebooks produced by DunneNote itself ([fixtures](fixtures/README.md)) | available |
| Test vectors for sibling positions and fold v1, for implementations in any language ([vectors](vectors/)) | available |

## Install

You need Rust ([rustup.rs](https://rustup.rs)); `rust-toolchain.toml` pins the version, and
rustup fetches it on first build.

```sh
git clone https://github.com/aacloudguy/dunnenote-format.git
cd dunnenote-format
cargo install --locked --path crates/dunnenote-cli    # puts dnfmt in ~/.cargo/bin
```

To use the library from Rust:

```toml
[dependencies]
dunnenote-format = { git = "https://github.com/aacloudguy/dunnenote-format", tag = "v0.1.0" }
```

## Try it

```sh
dnfmt inspect ~/Notes/Research.dunnenote
dnfmt ls ~/Notes/Research.dunnenote
dnfmt cat ~/Notes/Research.dunnenote <page-or-canvas-id>
dnfmt verify ~/Notes/Research.dunnenote --full

# Readable copies (the output folder must be new or empty; nothing is overwritten)
dnfmt export --md   ~/Notes/Research.dunnenote ~/Desktop/Research-md
dnfmt export --csv  ~/Notes/Research.dunnenote ~/Desktop/Research-csv
dnfmt export --json ~/Notes/Research.dunnenote ~/Desktop/Research.json
```

- **Markdown**: one `.md` file per page, in folders named after your sections, with pictures
  copied out and sketches drawn as SVG. Archived content is left out unless you add
  `--include-archived`.
- **CSV**: one file per table and one per calendar.
- **JSON**: everything in the notebook as one document (pictures and files are referenced by
  their SHA-256, not embedded). The document shape is versioned (`export_version`).

The reading commands open notebooks read-only and never take DunneNote's lock, so they are safe to
run while the notebook is open in DunneNote. They change nothing in the notebook. Like any SQLite
reader, they may leave the database's two standard companion files, `notebook.db-wal` (empty) and
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
dnfmt add-canvas ~/Notes/Trip.dunnenote "$P" calendar trip.ics     # events and attendees
T=$(dnfmt add-canvas ~/Notes/Trip.dunnenote "$P" table)             # an empty Editable table
dnfmt table ~/Notes/Trip.dunnenote "$T" add-row "Column 1=Tent" "Column 2=2"

F=$(dnfmt form ~/Notes/Trip.dunnenote new "$S" --name="Check-in")   # a form and its answers table
dnfmt form ~/Notes/Trip.dunnenote field <canvas-id> --name=Guest --required
dnfmt form ~/Notes/Trip.dunnenote submit "$F" "Guest=Ada Lovelace"  # appends one answers row

T=$(dnfmt template ~/Notes/Trip.dunnenote make "$P")                # "Day 1 (template)"
dnfmt template ~/Notes/Trip.dunnenote new "$T" "$S"                 # a new "Day 1" from it
dnfmt tag ~/Notes/Trip.dunnenote add "$P" "Travel/Italy"
dnfmt meta ~/Notes/Trip.dunnenote set "$P" "place=Florence"
dnfmt archive ~/Notes/Trip.dunnenote "$P" --reason=superseded      # retrieve undoes it
```

A form submitted with `dnfmt` or the library goes to an answers table. Forms that send their
answers to a document or a file are submitted in DunneNote, and a calendar field must have a day
chosen (its `displayDayEpoch`).

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

## Contributing and security

Bug reports, questions about the specification and pull requests to the library are welcome:
see [CONTRIBUTING.md](CONTRIBUTING.md). Please report security problems privately, as
[SECURITY.md](SECURITY.md) describes, not in a public issue.

## Licence

MIT — see [LICENSE](LICENSE). The specification, the schema, the test notebooks and the test
vectors are under the same licence.

DunneNote and DunneCorp are trademarks of Dunne, Corp. The licence covers this repository's
contents. It does not grant the right to use those names or logos, except to say truthfully
that software reads or writes DunneNote notebooks or follows this specification.
