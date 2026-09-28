# Golden notebooks

These notebooks were written by **DunneNote itself**, not by this library and not by hand. A
development-only program in the DunneNote repository builds each one through the same functions
the app's interface calls, then reads it back through the app's own read paths and records what
the app saw in [`expected.json`](expected.json).

`crates/dunnenote-format/tests/golden.rs` opens every notebook here and requires this library to
report the same thing, field for field.

| Notebook | What it covers |
| --- | --- |
| `minimal` | The smallest notebook: a root, one section, one empty page |
| `every-kind` | One canvas of every kind with content: rich text (headings, lists, marks, a link, a notebook link), sketch, picture with alt text, a markup layer and a caption, Data Table from CSV, Editable table, calendar from `.ics` (repeating and all-day events); nested sections and nested canvas groups |
| `forms` | Form fields (rich text and sketch), the answers table (`formTarget`), page form settings, one submission with the reserved `Submitted` column, and a hidden carrier canvas for a drawn answer |
| `template` | A page, the template made from it (placeholder picture, cleared and kept text), and a page made from the template |
| `archive` | An archived page (reason `superseded`), an archived canvas (reason `other` with a note), and an explicit minor layer |
| `tags` | Tags with aliases on a page, canvases, a table and a table row; typed metadata (text, date-time, location) |

Every string in these notebooks is synthetic. `.dunnenote.lock` files are not committed: DunneNote
creates the lock when it opens a notebook, and readers must not require it.

**Regenerating.** Ids are UUIDv7 and timestamps come from SQLite's clock, so every run produces
new ids. The notebooks and `expected.json` are therefore always replaced together, and only on
purpose: after a schema change, or to cover a new feature. From a DunneNote checkout:

```sh
cargo run -p dunnenote-api-host --example emit_golden -- /tmp/golden
rsync -a --delete --exclude .dunnenote.lock /tmp/golden/ fixtures/
```

Emitted by DunneNote 0.9.0, schema 18 (`format_version` 0.18.0). Licence: MIT, like the rest of
this repository.
