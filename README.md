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
| Library: write every canvas kind | planned |
| `dnfmt inspect`, `ls`, `cat`, `verify` | available (pre-release) |
| `dnfmt export` (Markdown, JSON, CSV) | planned |
| `dnfmt new`, `add-*`, `tag`, `archive`, `form submit` | planned |
| `SPEC.md` — full specification with a writer checklist | in progress |
| Golden test notebooks produced by DunneNote itself | planned |

## Try it

```sh
cargo build --release
./target/release/dnfmt inspect ~/Notes/Research.dunnenote
./target/release/dnfmt ls ~/Notes/Research.dunnenote
./target/release/dnfmt cat ~/Notes/Research.dunnenote <page-or-canvas-id>
./target/release/dnfmt verify ~/Notes/Research.dunnenote --full
```

`dnfmt` opens notebooks read-only and never takes DunneNote's lock, so it is safe to run while
the notebook is open in DunneNote.

## Supported versions

This library reads and writes notebooks at **schema 18** (`format_version` `0.18.0`). Notebooks
from newer DunneNote releases open read-only (one version ahead) or are refused; older notebooks
open read-only and should be opened once in DunneNote to upgrade.

## Licence

MIT — see [LICENSE](LICENSE). The specification, the schema and the test notebooks are under the
same licence.
